# MPFree's kernel build container (own-kernel step 2, docs/dev/hardware/own-kernel.md), used
# where Docker or Podman works. Debian trixie as published on 2026-11-04, pinned by digest, with
# its packages from that day's snapshot.debian.org, so every build in it has the same host tools.
# The kernel's compiler is not from here: it is kernel.org's crosstool gcc, pinned in versions.env,
# the same one build.sh uses where no container engine works.
# tools/kernel/build.sh builds it and tags it with this file's hash.
FROM docker.io/library/debian:trixie-20261104-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

ARG SNAPSHOT=20261104T000000Z

RUN printf '%s\n' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian/${SNAPSHOT}" \
      'Suites: trixie trixie-updates' \
      'Components: main' \
      'Check-Valid-Until: no' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      '' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian-security/${SNAPSHOT}" \
      'Suites: trixie-security' \
      'Components: main' \
      'Check-Valid-Until: no' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      > /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Retries=3 update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      bc bison ca-certificates cpio curl flex gcc gnupg kmod libc6-dev libelf-dev libssl-dev \
      lz4 make patch perl python3 xz-utils zstd \
 && rm -rf /var/lib/apt/lists/* \
 && echo "debian trixie-20261104-slim, snapshot ${SNAPSHOT}" > /etc/mpfree-kernel-build
