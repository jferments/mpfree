#!/bin/sh
# MPFree boot-chain and recovery map - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --boot-map.
# Reads block devices only through `dd if=... | hash` or `| tr` pipelines (never
# an output file), reads sysfs/procfs/unit text, and lists file NAMES.
# It NEVER mounts, remounts, writes partitions, edits the U-Boot environment,
# loads a kernel, changes services, or opens audio/display/input devices.
set -u
printf 'MPFREE BOOT/RECOVERY MAP - READ ONLY\n'
printf 'Raw output may contain MAC addresses and serials; redact before sharing.\n'

hash_cmd=''
if command -v sha256sum >/dev/null 2>&1; then hash_cmd='sha256sum'
elif command -v md5sum >/dev/null 2>&1; then hash_cmd='md5sum'; fi
printf 'Hash tool: %s\n' "${hash_cmd:-NONE}"

section() { printf '\n=== %s ===\n' "$1"; }
redact() {
  sed -E 's/((ethaddr|eth1addr|serial#|serialno|serial|androidboot\.serialno|cpuid#)[=:][[:space:]]*)[^[:space:]]+/\1<REDACTED>/g'
}

section 'Kernel and command line'
cat /proc/version 2>/dev/null || :
redact < /proc/cmdline 2>/dev/null || :

section 'Partition table (sysfs, names and geometry only)'
cat /proc/partitions 2>/dev/null || :
for dev in /sys/class/block/mmcblk0 /sys/class/block/mmcblk0p* \
           /sys/class/block/mmcblk0boot0 /sys/class/block/mmcblk0boot1 \
           /sys/class/block/mmcblk1 /sys/class/block/mmcblk1p*; do
  [ -d "$dev" ] || continue
  name="$(basename "$dev")"
  pname="$(sed -n 's/^PARTNAME=//p' "$dev/uevent" 2>/dev/null)"
  printf '%s partname=%s start=%s size_sectors=%s ro=%s' "$name" "${pname:--}" \
    "$(cat "$dev/start" 2>/dev/null || echo -)" "$(cat "$dev/size" 2>/dev/null || echo -)" \
    "$(cat "$dev/ro" 2>/dev/null || echo -)"
  if [ -r "$dev/force_ro" ]; then printf ' force_ro=%s' "$(cat "$dev/force_ro")"; fi
  printf '\n'
done
for f in /sys/block/mmcblk0/device/name /sys/block/mmcblk0/device/type \
         /sys/block/mmcblk0/device/manfid /sys/block/mmcblk0/device/rev \
         /sys/block/mmcblk0/device/fwrev; do
  [ -r "$f" ] && printf '%s: %s\n' "$f" "$(cat "$f")"
done

section 'eMMC write counters (field 7 = sectors written since boot)'
for dev in /sys/class/block/mmcblk0 /sys/class/block/mmcblk0p* \
           /sys/class/block/mmcblk0boot0 /sys/class/block/mmcblk0boot1; do
  [ -r "$dev/stat" ] || continue
  printf '%s %s\n' "$(basename "$dev")" "$(cat "$dev/stat")"
done

section 'Boot-critical partition hashes (read via dd, no output file)'
if [ -n "$hash_cmd" ]; then
  for name in mmcblk0p1 mmcblk0p2 mmcblk0p3 mmcblk0p4 mmcblk0p5 mmcblk0boot0 mmcblk0boot1 mmcblk0p6; do
    [ -b "/dev/$name" ] || { echo "$name: no block device"; continue; }
    printf '%s ' "$name"
    dd if="/dev/$name" bs=1048576 2>/dev/null | $hash_cmd | awk '{print $1}'
  done
else
  echo 'No hash tool available'
fi

section 'Rockchip loader signatures (first 16 bytes only)'
# BusyBox od on az0x lacks -A/-t; use -x (16-bit little-endian words) and -c.
for name in mmcblk0p1 mmcblk0p3 mmcblk0boot0 mmcblk0boot1; do
  [ -b "/dev/$name" ] || continue
  echo "--- $name ---"
  dd if="/dev/$name" bs=16 count=1 2>/dev/null | od -x | head -n 1
  dd if="/dev/$name" bs=16 count=1 2>/dev/null | od -c | head -n 1
done
echo '--- whole-disk sector 64 (Rockchip idbloader offset) ---'
dd if=/dev/mmcblk0 bs=512 skip=64 count=1 2>/dev/null | od -x | head -n 1

section 'U-Boot environment (p2) as text, identifiers redacted'
# Raw read of p2: a 4-byte CRC, then NUL-separated name=value pairs.
# No tr character classes: the device BusyBox may not support them.
if [ -b /dev/mmcblk0p2 ]; then
  echo '--- raw p2 (CRC bytes may prefix the first line) ---'
  dd if=/dev/mmcblk0p2 bs=8192 count=1 2>/dev/null | tr '\000' '\n' |
    grep -E '[A-Za-z0-9_#.-]+=' | redact | head -n 200
fi
echo '--- partuuid symlinks (to match fw_env.config to a partition) ---'
ls -l /dev/disk/by-partuuid/ 2>/dev/null || :
[ -r /etc/fw_env.config ] && { echo '/etc/fw_env.config:'; cat /etc/fw_env.config; }
echo '--- fw_printenv (read-only print of the same environment) ---'
if command -v fw_printenv >/dev/null 2>&1; then
  fw_printenv 2>&1 | redact | head -n 200
else
  echo 'fw_printenv: absent'
fi

section 'U-Boot boot script /boot/boot.scr.uimg (text after the 72-byte uImage script header)'
if [ -r /boot/boot.scr.uimg ]; then
  dd if=/boot/boot.scr.uimg bs=72 skip=1 2>/dev/null | tr -d '\000' | redact
  echo
fi

section 'Kernel and boot files in rootfs /boot (names and sizes)'
if [ -d /boot ]; then
  ls -la /boot 2>/dev/null || :
  find /boot -maxdepth 3 \( -name 'extlinux.conf' -o -name 'boot.scr' -o -name '*.its' \) -print 2>/dev/null
  for f in /boot/extlinux/extlinux.conf /boot/extlinux.conf; do
    [ -r "$f" ] && { echo "--- $f ---"; redact < "$f"; }
  done
else
  echo '/boot absent'
fi

section 'Update path (metadata only; nothing is mounted or executed)'
if [ -f /opt/ramfs-update.squashfs ]; then
  ls -la /opt/ramfs-update.squashfs
  [ -n "$hash_cmd" ] && $hash_cmd /opt/ramfs-update.squashfs
  printf 'squashfs magic (expect h s q s): '; dd if=/opt/ramfs-update.squashfs bs=4 count=1 2>/dev/null | od -c | head -n 1
fi
ls -la /opt 2>/dev/null || :
for d in /lib/systemd/system /usr/lib/systemd/system /etc/systemd/system; do
  [ -d "$d" ] || continue
  for u in "$d"/*update* "$d"/*Update* "$d"/*recovery* "$d"/*firmware*; do
    [ -f "$u" ] || continue
    echo "--- $u ---"; head -n 60 "$u" 2>/dev/null
  done
done
ls /usr/bin /usr/sbin /sbin 2>/dev/null | grep -i -E 'update|recover|flash|rkflash|upgrade' || :

section 'kexec support (no kernel is loaded)'
for f in /sys/kernel/kexec_loaded /sys/kernel/kexec_crash_loaded /sys/kernel/kexec_crash_size; do
  [ -r "$f" ] && printf '%s: %s\n' "$f" "$(cat "$f")"
done
command -v kexec 2>/dev/null || echo 'kexec binary: absent'
if [ -r /proc/config.gz ] && command -v zcat >/dev/null 2>&1; then
  zcat /proc/config.gz | grep -E '^(# )?CONFIG_(KEXEC|KEXEC_FILE|CRASH_DUMP|ATAGS_PROC|MMC_BLOCK_MINORS|EFI|IKCONFIG_PROC)\b' || :
else
  echo '/proc/config.gz unavailable'
fi

section 'Device tree: boot-related nodes'
for f in /proc/device-tree/model /proc/device-tree/compatible \
         /proc/device-tree/chosen/bootargs /proc/device-tree/chosen/stdout-path; do
  [ -r "$f" ] && { printf '%s: ' "$f"; tr '\000' ' ' < "$f" | redact; echo; }
done
if [ -d /proc/device-tree/aliases ]; then
  for f in /proc/device-tree/aliases/mmc* /proc/device-tree/aliases/usb*; do
    [ -r "$f" ] && printf '%s: %s\n' "$(basename "$f")" "$(tr -d '\000' < "$f")"
  done
fi

section 'USB OTG controller (MaskROM port candidate)'
for n in /proc/device-tree/usb@ff580000 /proc/device-tree/*/usb@ff580000; do
  [ -d "$n" ] || continue
  for p in compatible dr_mode status; do
    [ -r "$n/$p" ] && printf '%s/%s: %s\n' "$n" "$p" "$(tr '\000' ' ' < "$n/$p")"
  done
done
for u in /sys/class/udc/*; do
  [ -e "$u" ] || continue
  printf 'UDC %s state=%s\n' "$(basename "$u")" "$(cat "$u/state" 2>/dev/null || echo -)"
  readlink -f "$u/device" 2>/dev/null || :
done

section 'Recorded MPFree device modifications (see docs/hardware/DEVICE_MODIFICATIONS.md)'
for f in /usr/lib/systemd/system/mpfree-sshd.service \
         /usr/lib/systemd/system/multi-user.target.wants/mpfree-sshd.service; do
  if [ -e "$f" ] || [ -L "$f" ]; then
    ls -la "$f"
    [ -L "$f" ] && printf 'link -> %s\n' "$(readlink "$f")"
    [ -n "$hash_cmd" ] && [ -f "$f" ] && $hash_cmd "$f"
  else
    echo "absent: $f"
  fi
done
if command -v systemctl >/dev/null 2>&1; then
  printf 'mpfree-sshd enabled=%s active=%s\n' \
    "$(systemctl is-enabled mpfree-sshd.service 2>/dev/null || :)" \
    "$(systemctl is-active mpfree-sshd.service 2>/dev/null || :)"
fi
if [ -d /data/mpfree/ssh ]; then
  echo '/data/mpfree/ssh entries (names only, contents NOT read):'
  ls -1 /data/mpfree/ssh 2>/dev/null || :
fi

section 'Recent kernel boot messages about MMC/USB/boot (bounded)'
dmesg 2>/dev/null | grep -i -E 'mmc|emmc|dwc2|otg|rockchip.*boot|Machine model' | redact | head -n 60 || :

printf '\nEND MPFREE BOOT/RECOVERY MAP (no writes were requested)\n'
