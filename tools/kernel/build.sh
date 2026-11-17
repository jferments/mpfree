#!/usr/bin/env bash
# Build MPFree's kernel for the MPC One (own-kernel step 2, docs/dev/hardware/own-kernel.md):
# Linux 6.18 long-term with the PREEMPT_RT patch, the kernel's multi_v7_defconfig plus
# tools/kernel/mpfree.config, cross-compiled for 32-bit ARM with kernel.org's own cross compiler.
# Runs on the CI runners (.github/workflows/kernel.yml); works on any x86-64 Linux machine.
#
#   tools/kernel/build.sh OUT_DIR          build; OUT_DIR must not exist or be empty
#   tools/kernel/build.sh --probe          only print what this machine has
#
# 1. Prints what the machine has: system, CPUs, memory, disk, container engines, host tools, and
#    which hosts it reaches.
# 2. Picks where the build runs:
#    - in the pinned container (tools/kernel/Dockerfile: Debian pinned by digest, packages from one
#      day's snapshot.debian.org) when Docker or rootless Podman works for the user;
#    - else on the machine itself, as the user, never as root. A host tool it lacks (flex, bison,
#      m4, patch, bc, depmod, the OpenSSL headers) is fetched with "apt-get download", which checks
#      it against the machine's own signed package lists, and unpacked into this run's folder in
#      the cache. Nothing is installed.
#    The kernel is built by the same pinned compiler either way, never the machine's own.
# 3. Then ("build.sh --inside"):
#    - fetches the kernel, the RT patch and kernel.org's crosstool cross compiler (gcc and binutils
#      for arm-linux-gnueabi, the one kernel developers use) into the cache;
#    - checks them: against the sha256 pinned in versions.env; with nothing pinned, it takes the
#      newest 6.18 RT patch and its kernel and the newest gcc 14 crosstool, checks kernel.org's PGP
#      signatures (the kernel's must be Greg Kroah-Hartman's or Linus Torvalds's key, the compiler's
#      Arnd Bergmann's, who builds kernel.org's crosstool compilers; keys from kernel.org's pgpkeys
#      repository, else keys.openpgp.org by fingerprint) and prints the lines to pin;
#    - unpacks, patches, configures (multi_v7_defconfig + mpfree.config), checks the config
#      (check-config.py: every setting took, the eMMC and SD drivers are not built in), builds zImage
#      and the modules.
# 4. OUT_DIR gets zImage, modules-<release>.tar.xz, config, System.map, build-info.txt, SHA256SUMS.
#
# Writes nothing outside OUT_DIR, the cache (MPFREE_KERNEL_CACHE, default ~/.cache/mpfree-kernel:
# the downloads, and one folder per run for the build tree, removed when the run ends) and the
# container engine's image store.
# Environment: MPFREE_KERNEL_ENGINE (docker, podman or host; else the first container engine that
# works, else host), MPFREE_KERNEL_JOBS (default: the CPU count).
set -euo pipefail

say() { printf 'mpfree-kernel: %s\n' "$*"; }
die() { printf 'STOP: %s\n' "$*" >&2; exit 1; }

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

GREG_KH=647F28654894E3BD457199BE38DBBDC86092693E
LINUS=ABAF11C65A2970B130ABE3C479BE3E4300411886
CROSSTOOL_SIGNER=arnd@arndb.de
# Where the files come from. The tests point these at local folders; the checks below don't
# depend on them (a pinned sha256, or a signature by one of the keys above).
KORG=${MPFREE_KERNEL_KORG:-https://cdn.kernel.org/pub/linux/kernel}
CROSSTOOL=${MPFREE_KERNEL_CROSSTOOL:-https://cdn.kernel.org/pub/tools/crosstool/files/bin/x86_64}
PGPKEYS=${MPFREE_KERNEL_PGPKEYS:-https://git.kernel.org/pub/scm/docs/kernel/pgpkeys.git/plain/keys}
OPENPGP=${MPFREE_KERNEL_OPENPGP:-https://keys.openpgp.org/vks/v1/by-fingerprint}

# --------------------------------------------------------------------------------------------
# The build itself: in the container (/repo read-only, /cache, /out) or on the machine
# (MPFREE_INSIDE_REPO, MPFREE_INSIDE_CACHE, MPFREE_INSIDE_OUT). The tests run "--sources-only".
# --------------------------------------------------------------------------------------------

fetch() { # URL FILE: download once into the cache (a partial download never counts)
  [ -s "$2" ] && return 0
  local part
  part=$(mktemp "$2.part.XXXXXX")
  curl -fsSL --retry 3 --connect-timeout 30 -o "$part" "$1" || { rm -f "$part"; return 1; }
  mv -f "$part" "$2"
}

sha() { sha256sum "$1" | cut -d' ' -f1; }

pinned_ok() { # FILE PINNED_SHA256: the file is exactly the pinned one, else it leaves the cache
  local got
  got=$(sha "$1")
  [ "$got" = "$2" ] || { rm -f "$1"; die "$(basename "$1"): sha256 $got, pinned $2 (removed from the cache)"; }
}

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

verify_sig() { # XZFILE SIGFILE: the signature is over the uncompressed file; prints "PRIMARY-FPR UID"
  local status fpr uid
  status=$(xz -cd "$1" | gpg --status-fd 1 --verify "$2" - 2>/dev/null) || die "$(basename "$1"): kernel.org's PGP signature does not verify"
  # VALIDSIG <signing key> ... <primary key>: the primary key is the one people publish.
  fpr=$(printf '%s\n' "$status" | awk '$1 == "[GNUPG:]" && $2 == "VALIDSIG" { print ($12 != "" ? $12 : $3); exit }')
  uid=$(printf '%s\n' "$status" | sed -n 's/^\[GNUPG:\] GOODSIG [0-9A-F]* //p' | head -1)
  [ -n "$fpr" ] || die "$(basename "$1"): no good signature"
  printf '%s %s\n' "$fpr" "$uid"
}

checked_by_sig() { # FILE SIGURL...: fetch the signature (first URL that works), check it; prints "FPR UID (key from SOURCE)"
  local file="$1" sig="${1%.xz}.sign" k_src got u ok=no
  shift
  for u in "$@"; do fetch "$u" "$sig" && { ok=yes; break; }; done
  [ "$ok" = yes ] || die "could not fetch $(basename "$sig")"
  k_src=$(signer_key "$sig") || { rm -f "$sig"; exit 1; }
  got=$(verify_sig "$file" "$sig") || { rm -f "$file" "$sig"; exit 1; }
  printf '%s (key from %s)\n' "$got" "${k_src#* }"
}

key_uids() { gpg --with-colons --list-keys "$1" 2>/dev/null | awk -F: '$1 == "uid" { print $10 }'; }

inside() { # [--sources-only]
  local repo="${MPFREE_INSIDE_REPO:-/repo}" cache="${MPFREE_INSIDE_CACHE:-/cache}" out="${MPFREE_INSIDE_OUT:-/out}"
  local work tar patch gcc_tar gcc_name tarsum patchsum gccsum how_linux how_rt how_gcc pinned=yes
  # shellcheck disable=SC1091
  . "$repo/tools/kernel/versions.env"
  mkdir -p "$cache"
  # A run cut off before its end (a CI time-out) leaves its folder; no run takes a day.
  find "$cache" -maxdepth 1 \( -name 'run.*' -o -name '*.part.*' \) -mmin +1440 -exec rm -rf {} + 2>/dev/null || true
  work=$(mktemp -d "$cache/run.XXXXXX")
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" EXIT  # the build tree is several GB; nothing reads it after this run
  export GNUPGHOME="$work/gnupg"
  mkdir -m 700 "$GNUPGHOME"

  # The kernel and the RT patch.
  if [ -z "${LINUX_VERSION:-}" ]; then
    pinned=no
    say "no kernel pinned in versions.env: taking the newest $LINUX_SERIES RT patch on kernel.org"
    RT_PATCH=$(curl -fsSL --retry 3 "$KORG/projects/rt/$LINUX_SERIES/" \
      | grep -o "patch-$LINUX_SERIES\.[0-9]*-rt[0-9]*\.patch\.xz" | sort -uV | tail -1) || true
    [ -n "$RT_PATCH" ] || die "no patch-$LINUX_SERIES.*-rt*.patch.xz in $KORG/projects/rt/$LINUX_SERIES/"
    LINUX_VERSION=$(printf '%s\n' "$RT_PATCH" | sed 's/^patch-\(.*\)-rt[0-9]*\.patch\.xz$/\1/')
  elif [ -z "${RT_PATCH:-}" ] || [ -z "${LINUX_TAR_SHA256:-}" ] || [ -z "${RT_PATCH_SHA256:-}" ]; then
    die "versions.env pins LINUX_VERSION but not all of RT_PATCH, LINUX_TAR_SHA256 and RT_PATCH_SHA256"
  fi
  case "$LINUX_VERSION" in "$LINUX_SERIES".*) ;; *) die "LINUX_VERSION $LINUX_VERSION is not in $LINUX_SERIES" ;; esac
  say "kernel $LINUX_VERSION, RT patch $RT_PATCH"

  tar="$cache/linux-$LINUX_VERSION.tar.xz"
  patch="$cache/$RT_PATCH"
  fetch "$KORG/v${LINUX_SERIES%%.*}.x/linux-$LINUX_VERSION.tar.xz" "$tar" || die "could not fetch linux-$LINUX_VERSION.tar.xz"
  if ! fetch "$KORG/projects/rt/$LINUX_SERIES/$RT_PATCH" "$patch"; then
    fetch "$KORG/projects/rt/$LINUX_SERIES/older/$RT_PATCH" "$patch" || die "could not fetch $RT_PATCH"
  fi
  if [ -n "${LINUX_TAR_SHA256:-}" ]; then
    pinned_ok "$tar" "$LINUX_TAR_SHA256"; pinned_ok "$patch" "$RT_PATCH_SHA256"
    how_linux="sha256 pinned in versions.env"; how_rt="sha256 pinned in versions.env"
  else
    how_linux=$(checked_by_sig "$tar" "$KORG/v${LINUX_SERIES%%.*}.x/linux-$LINUX_VERSION.tar.sign")
    case "${how_linux%% *}" in
      "$GREG_KH"|"$LINUS") ;;
      *) rm -f "$tar"; die "linux-$LINUX_VERSION.tar.xz is signed by ${how_linux%% *}, not by Greg Kroah-Hartman or Linus Torvalds" ;;
    esac
    how_linux="PGP: $how_linux"
    how_rt="PGP: $(checked_by_sig "$patch" "$KORG/projects/rt/$LINUX_SERIES/${RT_PATCH%.xz}.sign" \
      "$KORG/projects/rt/$LINUX_SERIES/older/${RT_PATCH%.xz}.sign")"
  fi
  tarsum=$(sha "$tar"); patchsum=$(sha "$patch")
  say "linux-$LINUX_VERSION.tar.xz: $how_linux"
  say "$RT_PATCH: $how_rt"

  # The compiler: kernel.org's crosstool gcc and binutils for 32-bit ARM, made to run on any
  # x86-64 Linux, so the container and the machine build with exactly the same one.
  if [ -z "${GCC_VERSION:-}" ]; then
    pinned=no
    say "no compiler pinned in versions.env: taking the newest gcc $GCC_SERIES crosstool on kernel.org"
    GCC_VERSION=$(curl -fsSL --retry 3 "$CROSSTOOL/" \
      | grep -o "\"$GCC_SERIES\.[0-9]*\.[0-9]*/\"" | tr -d '"/' | sort -uV | tail -1) || true
    [ -n "$GCC_VERSION" ] || die "no gcc $GCC_SERIES.* folder in $CROSSTOOL/"
  elif [ -z "${GCC_TAR_SHA256:-}" ]; then
    die "versions.env pins GCC_VERSION but not GCC_TAR_SHA256"
  fi
  case "$GCC_VERSION" in "$GCC_SERIES".*) ;; *) die "GCC_VERSION $GCC_VERSION is not gcc $GCC_SERIES" ;; esac
  gcc_name="x86_64-gcc-$GCC_VERSION-nolibc-arm-linux-gnueabi.tar.xz"
  gcc_tar="$cache/$gcc_name"
  fetch "$CROSSTOOL/$GCC_VERSION/$gcc_name" "$gcc_tar" || die "could not fetch $CROSSTOOL/$GCC_VERSION/$gcc_name"
  if [ -n "${GCC_TAR_SHA256:-}" ]; then
    pinned_ok "$gcc_tar" "$GCC_TAR_SHA256"
    how_gcc="sha256 pinned in versions.env"
  else
    how_gcc=$(checked_by_sig "$gcc_tar" "$CROSSTOOL/$GCC_VERSION/${gcc_name%.xz}.sign")
    if ! key_uids "${how_gcc%% *}" | grep -qF "<$CROSSTOOL_SIGNER>"; then
      rm -f "$gcc_tar"
      die "$gcc_name is signed by ${how_gcc%% *} ($(key_uids "${how_gcc%% *}" | paste -sd ';' -)), not by Arnd Bergmann <$CROSSTOOL_SIGNER>, who builds kernel.org's crosstool compilers"
    fi
    how_gcc="PGP: $how_gcc"
  fi
  gccsum=$(sha "$gcc_tar")
  say "$gcc_name: $how_gcc"
  if [ "${1:-}" = --sources-only ]; then
    say "SOURCES-OK linux-$LINUX_VERSION $RT_PATCH gcc-$GCC_VERSION pinned=$pinned"
    return 0
  fi

  local build="$work/build" cc cross jobs release env_note
  mkdir -p "$build" "$work/toolchain"
  tar -xJf "$gcc_tar" -C "$work/toolchain"
  cc=$(find "$work/toolchain" -path '*/bin/arm-linux-gnueabi-gcc' | head -1)
  [ -n "$cc" ] || die "$gcc_name has no bin/arm-linux-gnueabi-gcc"
  cross="${cc%gcc}"
  "${cross}gcc" --version >/dev/null 2>&1 || die "the compiler in $gcc_name does not run on this machine"
  tar -xJf "$tar" -C "$build"
  local src="$build/linux-$LINUX_VERSION" obj="$build/obj" mods="$build/mods"
  xz -cd "$patch" | patch -d "$src" -p1 --batch --forward --quiet || die "$RT_PATCH does not apply to linux-$LINUX_VERSION"

  export ARCH=arm CROSS_COMPILE="$cross"
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
  [ -s "$mods/lib/modules/$release/modules.dep" ] || die "modules_install made no modules.dep (no depmod?)"

  cp "$obj/arch/arm/boot/zImage" "$out/zImage"
  cp "$obj/.config" "$out/config"
  cp "$obj/System.map" "$out/System.map"
  tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
      -C "$mods" -cJf "$out/modules-$release.tar.xz" "lib/modules/$release"
  env_note="${MPFREE_KERNEL_ENV:-unknown}"
  if [ -r /etc/mpfree-kernel-build ]; then env_note="$env_note, $(cat /etc/mpfree-kernel-build)"; fi
  {
    echo "release: $release"
    echo "linux: linux-$LINUX_VERSION.tar.xz sha256 $tarsum ($how_linux)"
    echo "rt: $RT_PATCH sha256 $patchsum ($how_rt)"
    echo "compiler: $gcc_name sha256 $gccsum ($how_gcc): $("${cross}gcc" --version | head -1)"
    echo "config: multi_v7_defconfig + tools/kernel/mpfree.config sha256 $(sha "$repo/tools/kernel/mpfree.config"); check-config: CONFIG-OK"
    echo "built in: $env_note"
    echo "source commit: $MPFREE_COMMIT (SOURCE_DATE_EPOCH $SOURCE_DATE_EPOCH)"
    echo "modules: $(find "$mods/lib/modules/$release" -name '*.ko*' | wc -l)"
    echo "zImage: $(stat -c %s "$out/zImage") bytes"
    if [ "$pinned" = no ]; then
      echo "pinned: no. To pin, put these in tools/kernel/versions.env:"
      echo "  LINUX_VERSION=$LINUX_VERSION"
      echo "  RT_PATCH=$RT_PATCH"
      echo "  LINUX_TAR_SHA256=$tarsum"
      echo "  RT_PATCH_SHA256=$patchsum"
      echo "  GCC_VERSION=$GCC_VERSION"
      echo "  GCC_TAR_SHA256=$gccsum"
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

os_name() { (. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-unknown}") || printf 'unknown'; }
has() { command -v "$1" >/dev/null 2>&1; }
has_depmod() { PATH="$PATH:/sbin:/usr/sbin" has depmod; }  # where the kernel's depmod.sh looks
openssl_headers() { printf '#include <openssl/bio.h>\n' | gcc ${HOSTCFLAGS:-} -E -x c - >/dev/null 2>&1; }
engine_works() { "$1" info >/dev/null 2>&1; }

pick_engine() {
  case "${MPFREE_KERNEL_ENGINE:-}" in
    host) echo host; return 0 ;;
    docker|podman) engine_works "$MPFREE_KERNEL_ENGINE" && { echo "$MPFREE_KERNEL_ENGINE"; return 0; }; return 1 ;;
    "") ;;
    *) return 1 ;;
  esac
  local e
  for e in docker podman; do
    has "$e" && engine_works "$e" && { echo "$e"; return 0; }
  done
  echo host
}

probe() {
  local e u code
  say "machine: $(os_name), $(uname -sm), $(nproc) CPUs, $(awk '/MemTotal/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo) memory"
  say "disk free in $HOME (the cache: $CACHE): $(df -h --output=avail "$HOME" 2>/dev/null | tail -1 | tr -d ' ')"
  for e in docker podman; do
    if ! has "$e"; then say "$e: not installed"
    elif engine_works "$e"; then say "$e: works ($("$e" --version 2>/dev/null | head -1))"
    else say "$e: installed, but '$e info' fails for $(id -un): $("$e" info 2>&1 | grep -v '^\s*$' | tail -1)"
    fi
  done
  for e in gcc make perl python3 patch flex bison m4 bc gpg apt-get dpkg-deb; do
    say "host tool $e: $(has "$e" && echo present || echo missing)"
  done
  say "host tool depmod: $(has_depmod && echo present || echo missing)"
  say "host OpenSSL headers: $(has gcc && openssl_headers && echo present || echo missing)"
  for u in https://cdn.kernel.org/ "$CROSSTOOL/" https://git.kernel.org/ https://keys.openpgp.org/ \
           http://snapshot.debian.org/ https://registry-1.docker.io/v2/; do
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "$u" 2>/dev/null) || true
    say "reach $u: HTTP ${code:-000}"
  done
}

host_packages() { # the Debian/Ubuntu packages with the host tools this machine lacks
  local pkgs="" t
  for t in flex bison patch bc; do has "$t" || pkgs="$pkgs $t"; done
  case "$pkgs" in *flex*|*bison*) has m4 || pkgs="$pkgs m4" ;; esac
  has_depmod || pkgs="$pkgs kmod"
  openssl_headers || pkgs="$pkgs libssl-dev"
  printf '%s\n' "${pkgs# }"
}

host_build() { # OUT COMMIT EPOCH: build on this machine, as this user
  local t u missing="" pkgs tools root arch lib env_note
  for t in gcc make perl python3 tar xz gzip curl gpg sha256sum; do has "$t" || missing="$missing $t"; done
  [ -z "$missing" ] || die "this machine lacks$missing. The build runs as $(id -un), without root; it fetches only flex, bison, m4, patch, bc, depmod and the OpenSSL headers itself"
  env_note="this machine as $(id -un), no container ($(os_name)); host tools: $(gcc --version | head -1), $(make --version | head -1)"
  pkgs=$(host_packages)
  if [ -n "$pkgs" ]; then
    has apt-get && has dpkg-deb || die "this machine lacks the tools in: $pkgs. Without apt-get and dpkg-deb the build can't fetch them; install them, or make Docker or Podman work"
    tools=$(mktemp -d "$CACHE/tools.XXXXXX")
    # shellcheck disable=SC2064
    trap "rm -rf '$tools'" EXIT
    say "fetching $pkgs with apt-get download, checked against this machine's signed package lists; nothing is installed"
    # shellcheck disable=SC2086
    (cd "$tools" && apt-get download -q $pkgs) || die "apt-get download $pkgs failed (above)"
    root="$tools/root"
    for t in "$tools"/*.deb; do
      dpkg-deb -x "$t" "$root"
      env_note="$env_note; fetched $(dpkg-deb -f "$t" Package) $(dpkg-deb -f "$t" Version)"
    done
    export PATH="$root/usr/bin:$root/usr/sbin:$root/bin:$root/sbin:$PATH"
    if [ -x "$root/usr/bin/m4" ]; then export M4="$root/usr/bin/m4"; fi
    if [ -d "$root/usr/share/bison" ]; then export BISON_PKGDATADIR="$root/usr/share/bison"; fi
    if [ -d "$root/usr/include/openssl" ]; then
      arch=$(gcc -dumpmachine)
      lib="$root/usr/lib/$arch"
      # libssl-dev's libcrypto.so points at libcrypto.so.N beside it, which is in the machine's
      # runtime package, not here: point it at the machine's own.
      t=$(readlink "$lib/libcrypto.so" 2>/dev/null || true)
      if [ -n "$t" ] && [ ! -e "$lib/$t" ]; then
        for u in "/usr/lib/$arch/$t" "/lib/$arch/$t"; do [ -e "$u" ] && { ln -sfn "$u" "$lib/libcrypto.so"; break; }; done
      fi
      export HOSTCFLAGS="-I$root/usr/include -I$root/usr/include/$arch" HOSTLDFLAGS="-L$lib"
    fi
    for t in flex bison patch bc; do has "$t" || die "$t is still missing after apt-get download"; done
    has_depmod || die "depmod is still missing after apt-get download"
    openssl_headers || die "the OpenSSL headers are still missing after apt-get download"
    printf '%%%%\n' | flex -t >/dev/null || die "flex does not run (above)"
  fi
  say "building on this machine as $(id -un), not in a container"
  MPFREE_INSIDE_REPO="$REPO" MPFREE_INSIDE_CACHE="$CACHE" MPFREE_INSIDE_OUT="$1" MPFREE_KERNEL_ENV="$env_note" \
    MPFREE_COMMIT="$2" SOURCE_DATE_EPOCH="$3" bash "$HERE/build.sh" --inside
}

main() {
  CACHE="${MPFREE_KERNEL_CACHE:-$HOME/.cache/mpfree-kernel}"
  case "${1:-}" in
    --inside) inside "${2:-}"; return ;;
    --probe) probe; return ;;
    ""|-*) die "usage: tools/kernel/build.sh OUT_DIR | --probe" ;;
  esac
  local out engine tag commit epoch run_as=()
  out="$(realpath -m -- "$1")"
  if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then die "$out is not empty"; fi
  probe
  engine=$(pick_engine) || die "MPFREE_KERNEL_ENGINE=$MPFREE_KERNEL_ENGINE does not work here (above); use docker, podman or host"
  mkdir -p "$out" "$CACHE"
  commit=$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)
  epoch=$(git -C "$REPO" log -1 --format=%ct 2>/dev/null || date +%s)
  if [ "$engine" = host ]; then
    host_build "$out" "$commit" "$epoch"
    return
  fi
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
    -e MPFREE_KERNEL_ENV="the container $tag ($engine)" \
    -e MPFREE_COMMIT="$commit" -e SOURCE_DATE_EPOCH="$epoch" \
    -v "$REPO:/repo:ro" -v "$CACHE:/cache" -v "$out:/out" \
    "$tag" bash /repo/tools/kernel/build.sh --inside
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
