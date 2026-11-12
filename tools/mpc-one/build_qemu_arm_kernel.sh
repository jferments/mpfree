#!/usr/bin/env bash
# Build a generic ARMv7 QEMU virt zImage; no root, no MPC, no device passthrough.
set -Eeuo pipefail
umask 077
WORK="$HOME/mpfree-work/mpfree-arm-qemu-lab"
SRC="$WORK/linux-arm-src"
OBJ="$WORK/linux-arm-build"
for tool in arm-linux-gnueabihf-gcc make tar python3; do
  command -v "$tool" >/dev/null || { echo "Missing $tool. Install cross-compiler/build packages." >&2; exit 1; }
done
shopt -s nullglob
# Ubuntu 24.04 ships 6.8; Ubuntu 26.04 (resolute) ships 7.0.
# Accept whichever *installed* version is present; do not fetch sources.
sources=(/usr/src/linux-source-[0-9]*/linux-source-[0-9]*.tar.*)
if (("${#sources[@]}" == 0)); then
  sources=(/usr/src/linux-source-[0-9]*.tar.*)
fi
(("${#sources[@]}" > 0)) || {
  echo "No installed kernel-source tarball found under /usr/src." >&2
  echo "Install the release-appropriate package: sudo apt install linux-source" >&2
  exit 1
}
mapfile -t sorted_sources < <(printf '%s\n' "${sources[@]}" | sort -V)
SOURCE_TARBALL="${sorted_sources[${#sorted_sources[@]}-1]}"
echo "Using installed kernel source: $SOURCE_TARBALL"
mkdir -p "$WORK" "$OBJ"
if [[ ! -f "$SRC/Makefile" ]]; then
  mkdir -p "$SRC"
  echo "Unpacking installed kernel source into $SRC ..."
  tar -xf "$SOURCE_TARBALL" -C "$SRC" --strip-components=1
fi
[[ -f "$SRC/arch/arm/configs/multi_v7_defconfig" ]] || { echo "Kernel source unpack invalid" >&2; exit 1; }
echo "Generating ARMv7 virt kernel config..."
make -s -C "$SRC" O="$OBJ" ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf- multi_v7_defconfig
"$SRC/scripts/config" --file "$OBJ/.config" \
  -e ARCH_VIRT -e OF -e OF_EARLY_FLATTREE \
  -e SERIAL_AMBA_PL011 -e SERIAL_AMBA_PL011_CONSOLE \
  -e VIRTIO -e VIRTIO_MMIO -e VIRTIO_BLK \
  -e EXT4_FS -e DEVTMPFS -e DEVTMPFS_MOUNT \
  -e BINFMT_ELF -e AEABI -e BLK_DEV -e TMPFS -e PROC_FS -e SYSFS \
  -e PCI -e PCI_HOST_GENERIC \
  -e UNIX -e INET \
  -d KEYSTONE_NAVIGATOR_QMSS -d KEYSTONE_NAVIGATOR_DMA \
  -d ARCH_KEYSTONE -d SOC_TI
# Linux 7.0 Ubuntu's multi_v7_defconfig selects TI Keystone QMSS,
# whose source currently fails to compile (knav_qdev undeclared).
# This driver is irrelevant to QEMU virt; disable it in the guest config.
make -s -C "$SRC" O="$OBJ" ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf- olddefconfig
# Critical drivers must be built in: Akai rootfs doesn't include matching module tree.
python3 - "$OBJ/.config" <<'PY'
from pathlib import Path
import sys
s=Path(sys.argv[1]).read_text()
keys=['ARCH_VIRT','SERIAL_AMBA_PL011','SERIAL_AMBA_PL011_CONSOLE',
      'VIRTIO_MMIO','VIRTIO_BLK','EXT4_FS','DEVTMPFS','BINFMT_ELF']
missing=[k for k in keys if 'CONFIG_'+k+'=y' not in s]
unwanted=['KEYSTONE_NAVIGATOR_QMSS','KEYSTONE_NAVIGATOR_DMA','ARCH_KEYSTONE']
still_enabled=[k for k in unwanted if 'CONFIG_'+k+'=y' in s]
if still_enabled:raise SystemExit('Unexpected non-QEMU Keystone drivers: '+', '.join(still_enabled))
if missing:raise SystemExit('Kernel config missing built-in drivers: '+', '.join(missing))
print('Verified core virt/PL011/virtio/ext4 drivers built in')
PY
JOBS="${MPFREE_QEMU_KERNEL_JOBS:-12}"
[[ "$JOBS" =~ ^[0-9]+$ ]] && ((JOBS>=1 && JOBS<=48)) || { echo "Invalid MPFREE_QEMU_KERNEL_JOBS" >&2; exit 1; }
echo "Building QEMU ARMv7 kernel (-j$JOBS)..."
make -C "$SRC" O="$OBJ" ARCH=arm CROSS_COMPILE=arm-linux-gnueabihf- -j"$JOBS" zImage
KERNEL="$OBJ/arch/arm/boot/zImage"
[[ -s "$KERNEL" ]] || { echo "No kernel produced" >&2; exit 1; }
sha256sum "$KERNEL" | tee "$WORK/virt-zImage.sha256"
echo "QEMU-compatible zImage built at: $KERNEL"
echo "This has not been booted yet and does NOT target RK3288 physical bootloader."