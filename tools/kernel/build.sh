#!/usr/bin/env bash
# Build MPFree's kernel for the MPC One (own-kernel step 2, docs/dev/hardware/own-kernel.md):
# Linux 6.18 long-term with the PREEMPT_RT patch, the kernel's multi_v7_defconfig plus
# tools/kernel/mpfree.config, cross-compiled for 32-bit ARM inside a pinned container.
# Runs on the CI runners (.github/workflows/kernel.yml); works on any x86-64 Linux machine
# where Docker or rootless Podman works for the user.
#
#   tools/kernel/build.sh OUT_DIR          build; OUT_DIR must not exist or be empty
#   tools/kernel/build.sh --probe          only print what this machine has
#
# 1. Prints what the machine has: container engines, CPUs, memory, disk, which hosts it reaches.
# 2. Builds the container from tools/kernel/Dockerfile (Debian pinned by digest, packages from one
#    day's snapshot.debian.org), tagged with the Dockerfile's hash; an engine reuses a built tag.
# 3. In the container ("build.sh --inside"):
#    - fetches the kernel and the RT patch from cdn.kernel.org into the cache;
#    - checks them: against the sha256 pinned in versions.env; with nothing pinned, it takes the
#      newest 6.18 RT patch and its kernel, checks kernel.org's PGP signatures (the kernel's must be
#      Greg Kroah-Hartman's or Linus Torvalds's key; keys from kernel.org's pgpkeys repository, else
#      keys.openpgp.org by fingerprint) and prints the lines to pin;
#    - unpacks, patches, configures (multi_v7_defconfig + mpfree.config), checks the config
#      (check-config.py: every setting took, the eMMC and SD drivers are not built in), builds zImage
#      and the modules.
# 4. OUT_DIR gets zImage, modules-<release>.tar.xz, config, System.map, build-info.txt, SHA256SUMS.
#
# Writes nothing outside OUT_DIR, the cache (MPFREE_KERNEL_CACHE, default ~/.cache/mpfree-kernel,
# where the build tree is remade on every run) and the container engine's image store.
# Environment: MPFREE_KERNEL_ENGINE (docker or podman, else the first that works),
# MPFREE_KERNEL_JOBS (default: the CPU count).
set -euo pipefail

say() { printf 'mpfree-kernel: %s\n' "$*"; }
die() { printf 'STOP: %s\n' "$*" >&2; exit 1; }

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

GREG_KH=647F28654894E3BD457199BE38DBBDC86092693E
LINUS=ABAF11C65A2970B130ABE3C479BE3E4300411886
# Where the files come from. The tests point these at local folders; the checks below don't
# depend on them (a pinned sha256, or a signature by one of the two keys above).
KORG=${MPFREE_KERNEL_KORG:-https://cdn.kernel.org/pub/linux/kernel}
PGPKEYS=${MPFREE_KERNEL_PGPKEYS:-https://git.kernel.org/pub/scm/docs/kernel/pgpkeys.git/plain/keys}
OPENPGP=${MPFREE_KERNEL_OPENPGP:-https://keys.openpgp.org/vks/v1/by-fingerprint}

# --------------------------------------------------------------------------------------------
# Inside the container: /repo (read-only), /cache, /out, HOME=/tmp/home. The tests run
# "--sources-only" outside it, with MPFREE_INSIDE_REPO and MPFREE_INSIDE_CACHE.
# --------------------------------------------------------------------------------------------

fetch() { # URL FILE: download once into the cache (a partial download never counts)
  [ -s "$2" ] && return 0
  curl -fsSL --retry 3 --connect-timeout 30 -o "$2.part" "$1" || { rm -f "$2.part"; return 1; }
  mv "$2.part" "$2"
}

sha() { sha256sum "$1" | cut -d' ' -f1; }

signer_key() { # SIGFILE: import the signer's key into GNUPGHOME; prints "FPR SOURCE"
  local sig="$1" fpr keyid src=""
  fpr=$(gpg --list-packets "$sig" 2>/dev/null | sed -n 's/.*issuer fpr v4 \([0-9A-F]\{40\}\).*/\1/p' | head -1)
  keyid=$(gpg --list-packets "$sig" 2>/dev/null | sed -n 's/.*keyid \([0-9A-F]\{16\}\).*/\1/p' | head -1)
  [ -n "$keyid" ] || [ -n "$fpr" ] || die "$sig: no signer key id in the signature"
  [ -n "$keyid" ] || keyid=${fpr:24}
  local key="$GNUPGHOME/signer.asc"
  if curl -fsSL --retry 2 -o "$key" "$PGPKEYS/$keyid.asc" 2>/dev/null && gpg -q --import "$key" 2>/dev/null; then
    src="kernel.org pgpkeys ($PGPKEYS/$keyid.asc)"
  elif [ -n "$fpr" ] && curl -fsSL --retry 2 -o "$key" "$OPENPGP/$fpr" 2>/dev/null && gpg -q --import "$key" 2>/dev/null; then
    src="keys.openpgp.org ($OPENPGP/$fpr)"
  else
    die "$sig: could not fetch the signer's key $keyid from kernel.org's pgpkeys or keys.openpgp.org"
  fi
  printf '%s %s\n' "${fpr:-$keyid}" "$src"
}

verify_sig() { # XZFILE SIGFILE: the signature is over the uncompressed file; prints "FPR UID"
  local status fpr uid
  status=$(xz -cd "$1" | gpg --status-fd 1 --verify "$2" - 2>/dev/null) || die "$1: kernel.org's PGP signature does not verify"
  fpr=$(printf '%s\n' "$status" | sed -n 's/^\[GNUPG:\] VALIDSIG \([0-9A-F]\{40\}\) .*/\1/p' | head -1)
  uid=$(printf '%s\n' "$status" | sed -n 's/^\[GNUPG:\] GOODSIG [0-9A-F]* //p' | head -1)
  [ -n "$fpr" ] || die "$1: no good signature"
  printf '%s %s\n' "$fpr" "$uid"
}

inside() { # [--sources-only]
  local repo="${MPFREE_INSIDE_REPO:-/repo}" cache="${MPFREE_INSIDE_CACHE:-/cache}" out=/out pinned=yes
  local build="$cache/build" tar patch tarsum patchsum how_linux how_rt jobs release
  # shellcheck disable=SC1091
  . "$repo/tools/kernel/versions.env"
  mkdir -p "$cache" "$HOME" && chmod 700 "$HOME"
  export GNUPGHOME="$HOME/gnupg"; mkdir -p "$GNUPGHOME"; chmod 700 "$GNUPGHOME"

  if [ -z "${LINUX_VERSION:-}" ]; then
    pinned=no
    say "nothing pinned in versions.env: taking the newest $LINUX_SERIES RT patch on kernel.org"
    RT_PATCH=$(curl -fsSL --retry 3 "$KORG/projects/rt/$LINUX_SERIES/" \
      | grep -o "patch-$LINUX_SERIES\.[0-9]*-rt[0-9]*\.patch\.xz" | sort -uV | tail -1) || true
    [ -n "$RT_PATCH" ] || die "no patch-$LINUX_SERIES.*-rt*.patch.xz in $KORG/projects/rt/$LINUX_SERIES/"
    LINUX_VERSION=$(printf '%s\n' "$RT_PATCH" | sed 's/^patch-\(.*\)-rt[0-9]*\.patch\.xz$/\1/')
  fi
  case "$LINUX_VERSION" in "$LINUX_SERIES".*) ;; *) die "LINUX_VERSION $LINUX_VERSION is not in $LINUX_SERIES" ;; esac
  say "kernel $LINUX_VERSION, RT patch $RT_PATCH"

  tar="$cache/linux-$LINUX_VERSION.tar.xz"
  patch="$cache/$RT_PATCH"
  fetch "$KORG/v${LINUX_SERIES%%.*}.x/linux-$LINUX_VERSION.tar.xz" "$tar" || die "could not fetch linux-$LINUX_VERSION.tar.xz"
  if ! fetch "$KORG/projects/rt/$LINUX_SERIES/$RT_PATCH" "$patch"; then
    fetch "$KORG/projects/rt/$LINUX_SERIES/older/$RT_PATCH" "$patch" || die "could not fetch $RT_PATCH"
  fi
  tarsum=$(sha "$tar"); patchsum=$(sha "$patch")

  if [ "$pinned" = yes ]; then
    [ "$tarsum" = "$LINUX_TAR_SHA256" ] || { rm -f "$tar"; die "linux-$LINUX_VERSION.tar.xz: sha256 $tarsum, pinned $LINUX_TAR_SHA256 (removed from the cache)"; }
    [ "$patchsum" = "$RT_PATCH_SHA256" ] || { rm -f "$patch"; die "$RT_PATCH: sha256 $patchsum, pinned $RT_PATCH_SHA256 (removed from the cache)"; }
    how_linux="sha256 pinned in versions.env"; how_rt="sha256 pinned in versions.env"
  else
    local sig_l="$cache/linux-$LINUX_VERSION.tar.sign" sig_r="$cache/${RT_PATCH%.xz}.sign" got k_src
    fetch "$KORG/v${LINUX_SERIES%%.*}.x/linux-$LINUX_VERSION.tar.sign" "$sig_l" || die "could not fetch linux-$LINUX_VERSION.tar.sign"
    fetch "$KORG/projects/rt/$LINUX_SERIES/${RT_PATCH%.xz}.sign" "$sig_r" \
      || fetch "$KORG/projects/rt/$LINUX_SERIES/older/${RT_PATCH%.xz}.sign" "$sig_r" || die "could not fetch ${RT_PATCH%.xz}.sign"
    k_src=$(signer_key "$sig_l"); got=$(verify_sig "$tar" "$sig_l")
    case "${got%% *}" in
      "$GREG_KH"|"$LINUS") ;;
      *) die "linux-$LINUX_VERSION.tar.xz is signed by ${got%% *}, not by Greg Kroah-Hartman or Linus Torvalds" ;;
    esac
    how_linux="PGP: ${got} (key from ${k_src#* })"
    k_src=$(signer_key "$sig_r"); got=$(verify_sig "$patch" "$sig_r")
    how_rt="PGP: ${got} (key from ${k_src#* })"
  fi
  say "linux-$LINUX_VERSION.tar.xz: $how_linux"
  say "$RT_PATCH: $how_rt"
  if [ "${1:-}" = --sources-only ]; then say "SOURCES-OK linux-$LINUX_VERSION $RT_PATCH pinned=$pinned"; return 0; fi

  rm -rf "$build"; mkdir -p "$build"
  trap 'rm -rf "$build"' EXIT  # the tree is several GB; nothing reads it after this run
  tar -xf "$tar" -C "$build"
  local src="$build/linux-$LINUX_VERSION" obj="$build/obj" mods="$build/mods"
  xz -cd "$patch" | patch -d "$src" -p1 --batch --forward --quiet || die "$RT_PATCH does not apply to linux-$LINUX_VERSION"

  export ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf-
  export KBUILD_BUILD_USER=mpfree KBUILD_BUILD_HOST=mpfree-ci
  export KBUILD_BUILD_TIMESTAMP; KBUILD_BUILD_TIMESTAMP="$(date -u -d "@$SOURCE_DATE_EPOCH" '+%Y-%m-%d %H:%M:%S UTC')"
  jobs="${MPFREE_KERNEL_JOBS:-$(nproc)}"
  make -s -C "$src" O="$obj" multi_v7_defconfig
  (cd "$src" && scripts/kconfig/merge_config.sh -m -O "$obj" "$obj/.config" "$repo/tools/kernel/mpfree.config") >/dev/null
  make -s -C "$src" O="$obj" olddefconfig
  python3 -I "$repo/tools/kernel/check-config.py" "$repo/tools/kernel/mpfree.config" "$obj/.config" \
    || die "the built config does not have MPFree's settings (above); fix tools/kernel/mpfree.config"
  say "building zImage and modules with $jobs jobs"
  make -s -C "$src" O="$obj" -j"$jobs" zImage modules
  make -s -C "$src" O="$obj" INSTALL_MOD_PATH="$mods" INSTALL_MOD_STRIP=1 modules_install
  release=$(cat "$obj/include/config/kernel.release")
  rm -f "$mods/lib/modules/$release/build" "$mods/lib/modules/$release/source"

  cp "$obj/arch/arm/boot/zImage" "$out/zImage"
  cp "$obj/.config" "$out/config"
  cp "$obj/System.map" "$out/System.map"
  tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
      -C "$mods" -cJf "$out/modules-$release.tar.xz" "lib/modules/$release"
  {
    echo "release: $release"
    echo "linux: linux-$LINUX_VERSION.tar.xz sha256 $tarsum ($how_linux)"
    echo "rt: $RT_PATCH sha256 $patchsum ($how_rt)"
    echo "config: multi_v7_defconfig + tools/kernel/mpfree.config sha256 $(sha "$repo/tools/kernel/mpfree.config"); check-config: CONFIG-OK"
    echo "container: $(cat /etc/mpfree-kernel-build); $MPFREE_KERNEL_IMAGE"
    echo "compiler: $(arm-linux-gnueabihf-gcc --version | head -1)"
    echo "source commit: $MPFREE_COMMIT (SOURCE_DATE_EPOCH $SOURCE_DATE_EPOCH)"
    echo "modules: $(find "$mods/lib/modules/$release" -name '*.ko*' | wc -l)"
    echo "zImage: $(stat -c %s "$out/zImage") bytes"
    if [ "$pinned" = no ]; then
      echo "pinned: no. To pin, put these in tools/kernel/versions.env:"
      echo "  LINUX_VERSION=$LINUX_VERSION"
      echo "  RT_PATCH=$RT_PATCH"
      echo "  LINUX_TAR_SHA256=$tarsum"
      echo "  RT_PATCH_SHA256=$patchsum"
    else
      echo "pinned: yes"
    fi
  } > "$out/build-info.txt"
  (cd "$out" && sha256sum zImage "modules-$release.tar.xz" config System.map build-info.txt > SHA256SUMS)
  cat "$out/build-info.txt"
  say "BUILD-OK $release"
}

# --------------------------------------------------------------------------------------------
# On the machine.
# --------------------------------------------------------------------------------------------

engine_works() { "$1" info >/dev/null 2>&1; }

pick_engine() {
  if [ -n "${MPFREE_KERNEL_ENGINE:-}" ]; then
    engine_works "$MPFREE_KERNEL_ENGINE" && { echo "$MPFREE_KERNEL_ENGINE"; return 0; }
    return 1
  fi
  local e
  for e in docker podman; do
    command -v "$e" >/dev/null 2>&1 && engine_works "$e" && { echo "$e"; return 0; }
  done
  return 1
}

probe() {
  local e u code
  say "machine: $(uname -sm), $(nproc) CPUs, $(awk '/MemTotal/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo) memory"
  say "disk free in $HOME (the cache: $CACHE): $(df -h --output=avail "$HOME" 2>/dev/null | tail -1 | tr -d ' ')"
  for e in docker podman; do
    if ! command -v "$e" >/dev/null 2>&1; then say "$e: not installed"
    elif engine_works "$e"; then say "$e: works ($("$e" --version 2>/dev/null | head -1))"
    else say "$e: installed, but '$e info' fails for $(id -un): $("$e" info 2>&1 | grep -v '^\s*$' | tail -1)"
    fi
  done
  for u in https://cdn.kernel.org/ https://git.kernel.org/ https://keys.openpgp.org/ http://snapshot.debian.org/ https://registry-1.docker.io/v2/; do
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "$u" 2>/dev/null) || true
    say "reach $u: HTTP ${code:-000}"
  done
  for e in arm-linux-gnueabihf-gcc bison flex bc make gpg; do
    say "host tool $e: $(command -v "$e" >/dev/null 2>&1 && echo present || echo missing)"
  done
}

main() {
  CACHE="${MPFREE_KERNEL_CACHE:-$HOME/.cache/mpfree-kernel}"
  case "${1:-}" in
    --inside) inside "${2:-}"; return ;;
    --probe) probe; return ;;
    ""|-*) die "usage: tools/kernel/build.sh OUT_DIR | --probe" ;;
  esac
  local out engine tag run_as=()
  out="$(realpath -m -- "$1")"
  if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then die "$out is not empty"; fi
  probe
  engine=$(pick_engine) || die "no container engine works here for $(id -un) (above). The owner's choice is a pinned container: Docker or rootless Podman must work for this user, or the toolchain is pinned on the host instead."
  mkdir -p "$out" "$CACHE"
  tag="mpfree-kernel-build:$(sha256sum "$HERE/Dockerfile" | cut -c1-12)"
  if ! "$engine" image inspect "$tag" >/dev/null 2>&1; then
    say "building the container $tag with $engine"
    "$engine" build -t "$tag" -f "$HERE/Dockerfile" "$HERE"
  fi
  case "$engine" in
    podman) run_as=(--userns=keep-id) ;;
    *) run_as=(-u "$(id -u):$(id -g)") ;;
  esac
  "$engine" run --rm "${run_as[@]}" \
    -e HOME=/tmp/home \
    -e MPFREE_KERNEL_JOBS="${MPFREE_KERNEL_JOBS:-}" \
    -e MPFREE_KERNEL_IMAGE="$tag ($engine)" \
    -e MPFREE_COMMIT="$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)" \
    -e SOURCE_DATE_EPOCH="$(git -C "$REPO" log -1 --format=%ct 2>/dev/null || date +%s)" \
    -v "$REPO:/repo:ro" -v "$CACHE:/cache" -v "$out:/out" \
    "$tag" bash /repo/tools/kernel/build.sh --inside
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
