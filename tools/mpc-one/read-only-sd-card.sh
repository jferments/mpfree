#!/bin/sh
# MPFree SD card survey - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --sd-card (Akai's system, port 22), with the
# owner's SD card in the slot. It is step S0 of the SD card plan
# (docs/dev/hardware/sd-card-and-internal-drive.md): how does Akai's system use an SD card, so
# MPFree can share one with it (the owner chose a folder on the card as it is)?
#
# Answers: which filesystems Akai's kernel reads, the SD slot's device-tree settings (card
# detect, write protect), the bus speed the card runs at, the card's size, partitions and
# filesystem (from udev's records), how Akai's system mounted it and with which options, the
# names at the top of the card, which disk tools Akai's system has (fsck, mkfs), Akai's disk
# service and the udev rules that act on disks, and what the kernel and Akai's services said
# about the card in this boot.
#
# It reads procfs, sysfs, the device tree, udev's records in /run/udev/data, debugfs only if it
# is already mounted, unit files, udev rules, /etc/fstab, the kernel log and the journal, and
# lists the names in the card's top folder when it is FAT or exFAT (whose top folder keeps no
# dates, so listing it changes nothing; any other filesystem's is not listed). It never mounts or unmounts, never runs blkid, fsck, mkfs or tune2fs,
# never opens a block device, never binds or unbinds a driver and never writes a file. It prints
# no serial number, MAC address or filesystem UUID.
set -u
R="${MPFREE_FAKE_ROOT:-}"
SD=ff0c0000
printf 'MPFREE SD CARD SURVEY - READ ONLY\n'
printf 'Serial numbers, MAC addresses and UUIDs are replaced with <serial>, <MAC> and <uuid>.\n'

section() { printf '\n=== %s ===\n' "$1"; }
redact() {
  sed -e 's/[0-9A-Fa-f][0-9A-Fa-f]\(:[0-9A-Fa-f][0-9A-Fa-f]\)\{5\}/<MAC>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{12\}/<uuid>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{2\}/<uuid>/g' \
      -e 's/\([^0-9A-Za-z-]\)[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}\([^0-9A-Za-z-]\)/\1<uuid>\2/g' \
      -e 's/\([^0-9A-Za-z-]\)[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}$/\1<uuid>/' \
      -e 's/^[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}\([^0-9A-Za-z-]\)/<uuid>\1/' \
      -e 's/\([A-Za-z_.]*[Ss][Ee][Rr][Ii][Aa][Ll][A-Za-z_.]*[=:] *\)[^ ,]*/\1<serial>/g'
}
value() { [ -r "$1" ] && head -n 1 "$1" 2>/dev/null; return 0; }
strings_in() { [ -r "$1" ] && tr '\000' ' ' < "$1" | sed 's/ *$//'; return 0; }
kernel_log() { if [ -n "$R" ]; then cat "$R/kernel-log" 2>/dev/null; else dmesg 2>/dev/null; fi; }
journal() { if [ -n "$R" ]; then cat "$R/journal" 2>/dev/null; else journalctl -b --no-pager -o short-monotonic 2>/dev/null; fi; }
# A mount point as /proc/mounts writes it (\040 for a space, \011 tab, \012 newline, \134 backslash).
unescape() { printf '%s' "$1" | sed -e 's/\\040/ /g' -e 's/\\011/	/g' -e 's/\\134/\\/g'; }
# udev's record of a block device: the filesystem and partition facts only, no UUIDs or serials.
udev_facts() {
  [ -r "$R/run/udev/data/b$1" ] || { printf '    (no udev record for %s)\n' "$1"; return 0; }
  grep -E '^E:ID_(FS_TYPE|FS_VERSION|FS_USAGE|FS_LABEL|PART_TABLE_TYPE|PART_ENTRY_TYPE|PART_ENTRY_NUMBER|DRIVE_FLASH_SD|DRIVE_MEDIA_FLASH_SD|NAME|TYPE|BUS)=' \
    "$R/run/udev/data/b$1" 2>/dev/null | sed 's/^E:/    /' | redact
  grep -E '^E:(UDISKS_[A-Z_]+|SYSTEMD_WANTS|SYSTEMD_READY)=' "$R/run/udev/data/b$1" 2>/dev/null \
    | sed 's/^E:/    /' | redact
  return 0
}

section 'Kernel'
printf 'release: %s\n' "$(value "$R/proc/sys/kernel/osrelease")"
printf 'filesystems it reads now (/proc/filesystems):'
for fs in vfat msdos exfat ntfs ntfs3 ext2 ext3 ext4 fuse fuseblk hfsplus btrfs f2fs; do
  grep -q "[[:space:]]$fs\$" "$R/proc/filesystems" 2>/dev/null && printf ' %s' "$fs"
done
printf '\n'
if [ -r "$R/proc/config.gz" ] && gzip -dc "$R/proc/config.gz" >/dev/null 2>&1; then
  printf 'build settings for disks (/proc/config.gz):\n'
  gzip -dc "$R/proc/config.gz" \
    | grep -E '^(# )?CONFIG_(VFAT_FS|MSDOS_FS|FAT_[A-Z_]+|EXFAT_[A-Z_]+|NTFS3?_[A-Z_]+|FUSE_FS|EXT4_FS|NLS_DEFAULT|NLS_CODEPAGE_437|NLS_ISO8859_1|NLS_UTF8|MMC|MMC_BLOCK[A-Z_]*|MMC_DW[A-Z_]*)(=| is not set)' \
    | sed 's/^/  /'
else
  printf 'build settings: no readable /proc/config.gz\n'
fi

section 'The SD slot (device tree and driver)'
found=no
for node in "$R"/sys/firmware/devicetree/base/*@"$SD" "$R"/sys/firmware/devicetree/base/*/*@"$SD"; do
  [ -d "$node" ] || continue
  found=yes
  printf 'node: /%s\n' "${node#"$R"/sys/firmware/devicetree/base/}"
  printf '  compatible: %s\n' "$(strings_in "$node/compatible")"
  printf '  status: %s\n' "$(strings_in "$node/status")"
  printf '  properties:'
  for p in "$node"/*; do [ -f "$p" ] && printf ' %s' "$(basename "$p")"; done
  printf '\n'
  for p in cd-gpios broken-cd non-removable cd-inverted disable-wp wp-gpios wp-inverted \
           cap-sd-highspeed sd-uhs-sdr12 sd-uhs-sdr25 sd-uhs-sdr50 sd-uhs-sdr104 sd-uhs-ddr50; do
    if [ -e "$node/$p" ]; then printf '  %s: yes\n' "$p"; else printf '  %s: no\n' "$p"; fi
  done
done
[ "$found" = yes ] || printf 'node: none at %s in the device tree\n' "$SD"
host=none
for dev in "$R"/sys/bus/platform/devices/"$SD".*; do
  [ -e "$dev" ] || continue
  drv=none
  [ -L "$dev/driver" ] && drv=$(basename "$(readlink "$dev/driver")")
  printf 'platform device: %s, driver: %s\n' "$(basename "$dev")" "$drv"
  for h in "$dev"/mmc_host/mmc*; do [ -e "$h" ] && host=$(basename "$h"); done
done
printf 'MMC host: %s\n' "$host"
if [ "$host" != none ]; then
  if grep -q ' /sys/kernel/debug debugfs ' "$R/proc/mounts" 2>/dev/null; then
    if [ -r "$R/sys/kernel/debug/$host/ios" ]; then
      printf 'bus settings now (debugfs %s/ios):\n' "$host"
      sed 's/^/  /' "$R/sys/kernel/debug/$host/ios"
    else
      printf 'bus settings: no readable debugfs %s/ios\n' "$host"
    fi
  else
    printf 'bus settings: debugfs is not mounted; this script does not mount it\n'
  fi
fi

section 'The card'
card=none
for b in "$R"/sys/block/mmcblk*; do
  [ -d "$b" ] || continue
  case "$(readlink "$b" 2>/dev/null)" in *"$SD"*) card=$(basename "$b") ;; esac
done
if [ "$card" = none ]; then
  printf 'card: no block device under the SD controller (no card, or the controller is not bound)\n'
else
  b="$R/sys/block/$card"
  sectors=$(value "$b/size")
  printf 'card: %s (%s), %s sectors (%s GiB), read-only %s, force_ro %s, removable %s\n' "$card" \
    "$(value "$b/dev")" "$sectors" "$(awk -v s="${sectors:-0}" 'BEGIN { printf "%.1f", s * 512 / 1073741824 }')" \
    "$(value "$b/ro")" "$(value "$b/force_ro")" "$(value "$b/removable")"
  for f in type name manfid oemid date hwrev fwrev; do
    [ -r "$b/device/$f" ] && printf '  %s: %s\n' "$f" "$(value "$b/device/$f")"
  done
  printf '  udev:\n'
  udev_facts "$(value "$b/dev")"
  for p in "$b"/"$card"p*; do
    [ -d "$p" ] || continue
    printf 'partition %s (%s): start %s, %s sectors, read-only %s\n' "$(basename "$p")" "$(value "$p/dev")" \
      "$(value "$p/start")" "$(value "$p/size")" "$(value "$p/ro")"
    udev_facts "$(value "$p/dev")"
  done
fi

section 'Mounted from the card (/proc/mounts)'
mounted=no
if [ "$card" != none ]; then
  while read -r dev dir type opts _; do
    case "$dev" in
      /dev/"$card"*)
        mounted=yes
        printf '%s on %s type %s (%s)\n' "$dev" "$dir" "$type" "$opts" | redact
        top="$R$(unescape "$dir")"
        case "$type" in
          vfat|msdos|exfat)
            if [ -d "$top" ]; then
              printf '  top folder: %s names\n' "$(ls -1A "$top" 2>/dev/null | grep -c '')"
              ls -1Ap "$top" 2>/dev/null | head -n 60 | sed 's/^/    /'
            fi ;;
          *) printf '  top folder: not listed (a %s top folder keeps an access date, which listing could change)\n' "$type" ;;
        esac ;;
    esac
  done < "$R/proc/mounts"
fi
[ "$mounted" = yes ] || printf 'nothing from the card is mounted\n'
printf '/etc/fstab lines for MMC disks or /media:\n'
grep -E 'mmcblk|/media' "$R/etc/fstab" 2>/dev/null | redact | sed 's/^/  /'

section 'Disk tools on this system'
for t in blkid fsck fsck.vfat fsck.fat dosfsck fsck.exfat exfatfsck mkfs.vfat mkfs.fat mkfs.exfat ntfsfix ntfs-3g \
         mount.exfat mount.exfat-fuse tune2fs e2fsck; do
  p=$(command -v "$t" 2>/dev/null) || p=missing
  printf '%s: %s\n' "$t" "$p"
done

section "Akai's disk service and udev rules for disks"
for u in "$R"/lib/systemd/system/*edisk* "$R"/usr/lib/systemd/system/*edisk* "$R"/etc/systemd/system/*edisk* \
         "$R"/lib/systemd/system/no-reserved-blocks@.service "$R"/usr/lib/systemd/system/no-reserved-blocks@.service; do
  [ -f "$u" ] || continue
  printf -- '--- %s\n' "${u#"$R"}"
  grep -v -E '^[[:space:]]*(#|$)' "$u" | redact
done
for d in "$R"/lib/udev/rules.d "$R"/usr/lib/udev/rules.d "$R"/etc/udev/rules.d; do
  [ -d "$d" ] || continue
  grep -H -n -i -E 'mmcblk|edisk|no-reserved|ID_FS_TYPE|SYSTEMD_WANTS' "$d"/*.rules 2>/dev/null \
    | sed "s|^$R||" | redact
done

section 'What the kernel said about the SD card (this boot)'
pattern="$SD|exfat|fat-fs|ntfs"
[ "$host" = none ] || pattern="$pattern|$host:|$host "
[ "$card" = none ] || pattern="$pattern|$card"
kernel_log | grep -i -E "$pattern" | tail -n 80 | redact

section "What Akai's services said about disks (this boot's journal)"
pattern='edisksd|no-reserved-blocks|/media/'
[ "$card" = none ] || pattern="$pattern|$card"
journal | grep -i -E "$pattern" | tail -n 120 | redact

printf '\nEND OF SD CARD SURVEY\n'
