#!/bin/sh
# MPC One userspace ABI inventory. Read-only: no binaries copied or executed,
# apart from ordinary installed query tools; no library files modified.
set -u
printf 'MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY\n'
printf '=== Kernel and CPU ===\n'
uname -a 2>&1 || :
if [ -r /proc/cpuinfo ]; then
  grep -E '^(Processor|model name|Features|CPU architecture|CPU part)' /proc/cpuinfo | head -n 20 || :
fi
printf '\n=== Operating system release ===\n'
if [ -r /etc/os-release ]; then cat /etc/os-release; fi
printf '\n=== Runtime GNU libc version ===\n'
if command -v getconf >/dev/null 2>&1; then
  printf 'getconf GNU_LIBC_VERSION: '
  getconf GNU_LIBC_VERSION 2>&1 || :
else
  printf 'getconf not found (libc version remains unverified)\n'
fi
printf '\n=== GNU libc embedded release banner (read-only strings) ===\n'
if command -v strings >/dev/null 2>&1 && [ -r /lib/libc.so.6 ]; then
  banner="$(strings /lib/libc.so.6 2>/dev/null | \
    grep -E '^GNU C Library.*release version [0-9]+\.[0-9]+' | head -n 1 || :)"
  if [ -n "$banner" ]; then
    printf 'GNU libc release banner: %s\n' "$banner"
  else
    printf 'GNU libc release banner: unavailable (do not infer version)\n'
  fi
else
  printf 'GNU libc release banner: unavailable (strings/libc missing)\n'
fi
printf '\n=== Dynamic ELF interpreter and known native library paths ===\n'
for path in /lib/ld-linux-armhf.so.3 /lib/libc.so.6 \
  /lib/libm.so.6 /lib/libpthread.so.0 \
  /usr/lib/libasound.so* /lib/libasound.so* \
  /usr/lib/libdrm.so* /lib/libdrm.so* \
  /usr/lib/libEGL.so* /lib/libEGL.so* \
  /usr/lib/libGLESv2.so* /lib/libGLESv2.so* \
  /usr/lib/libgbm.so* /lib/libgbm.so* \
  /usr/lib/libstdc++.so* /lib/libstdc++.so*; do
  [ -e "$path" ] || [ -L "$path" ] || continue
  printf '%s -> ' "$path"
  if [ -L "$path" ]; then readlink "$path" 2>/dev/null || :; else echo '(regular file)'; fi
done
printf '\n=== ELF header (first 64 bytes, no factory binary copied) ===\n'
if [ -r /usr/bin/MPC ] && command -v dd >/dev/null 2>&1 && \
   command -v od >/dev/null 2>&1; then
  dd if=/usr/bin/MPC bs=64 count=1 2>/dev/null | od -b
fi
printf '\n=== Available linker and interpreter tools ===\n'
for cmd in readelf objdump ldd ldconfig pkg-config strings; do
  if command -v "$cmd" >/dev/null 2>&1; then
    command -v "$cmd"
  else
    printf '%s not installed\n' "$cmd"
  fi
done
printf '\n=== Available GLIBC symbol tag maximum, if strings exists ===\n'
# This reads installed libc *as data*, not its init or loader entry points.
if command -v strings >/dev/null 2>&1 && [ -r /lib/libc.so.6 ]; then
  strings /lib/libc.so.6 2>/dev/null | \
    grep -E '^GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?$' | \
    sort -Vu | tail -n 5 || :
else
  echo 'GLIBC tags not available'
fi
printf '\nEND MPFREE ARMV7 ABI SURVEY\n'
