R=''
# MPFree streaming survey - STRICTLY READ-ONLY. Step D0 of the disk streaming plan
# (docs/dev/design/disk-streaming.md).
#
# host_disktest_mpc.py streams it over SSH stdin to the test build (MPFree's system on the USB
# stick, port 2222, checked against the stick build's host key), in its step 2. It records what
# decides how steadily partition 4 can stream audio: the kernel, how partition 4 is mounted and
# how full it is, the stick's block queue (scheduler, request size, read-ahead, write cache), the
# USB path to it (speed, the storage driver, the host controller), the page cache's writeback
# settings for that disk, memory, the USB interrupts and the threads that serve them (CPU,
# scheduling policy, priority), the pad player's real-time threads, the CPUs and their
# governors, the disk's I/O counters, which file systems the kernel reads for the SD plan, what
# else from a disk is mounted and whether Akai's disk service runs, and what the kernel said
# about the stick in this boot.
#
# It reads procfs, sysfs, unit files and the kernel log only. It never mounts or unmounts,
# never opens a block device, never runs blkid, fsck, mkfs, tune2fs, hdparm or sysctl, never
# writes a file and changes no setting. It prints no serial number, MAC address or UUID.
set -u
printf 'MPFREE STREAM SURVEY - READ ONLY\n'
printf 'Serial numbers, MAC addresses and UUIDs are replaced with <serial>, <MAC> and <uuid>.\n'

section() { printf '\n=== %s ===\n' "$1"; }
value() { [ -r "$1" ] && head -n 1 "$1" 2>/dev/null; return 0; }
redact() {
  sed -e 's/[0-9A-Fa-f][0-9A-Fa-f]\(:[0-9A-Fa-f][0-9A-Fa-f]\)\{5\}/<MAC>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{12\}/<uuid>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{2\}/<uuid>/g' \
      -e 's/\([^0-9A-Za-z-]\)[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}\([^0-9A-Za-z-]\)/\1<uuid>\2/g' \
      -e 's/\([^0-9A-Za-z-]\)[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}$/\1<uuid>/' \
      -e 's/^[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}\([^0-9A-Za-z-]\)/<uuid>\1/' \
      -e 's/\([A-Za-z_.]*[Ss][Ee][Rr][Ii][Aa][Ll][A-Za-z_.]*[=:] *\)[^ ,]*/\1<serial>/g'
}
# Each named file in a folder, one per line: "  name: first line".
files() {
  d=$1; shift
  for f in "$@"; do
    [ -r "$d/$f" ] && printf '  %s: %s\n' "$f" "$(value "$d/$f")"
  done
  return 0
}
kernel_log() { if [ -n "$R" ]; then cat "$R/kernel-log" 2>/dev/null; else dmesg 2>/dev/null; fi; }
policy_name() {
  case "$1" in
    0) echo other ;; 1) echo FIFO ;; 2) echo RR ;; 3) echo batch ;; 5) echo idle ;; 6) echo deadline ;; *) echo "$1" ;;
  esac
}
# A task's scheduling from its stat and status files: "policy P, priority N, last CPU C, CPUs L".
# Fields after "(comm) ": $1 is the state (field 3), so field N is ${N-2}.
sched() {
  s=$(cat "$1/stat" 2>/dev/null) || return 0
  rest=${s##*) }
  # shellcheck disable=SC2086
  set -- $rest
  printf 'policy %s, priority %s, last CPU %s' "$(policy_name "${39}")" "${38}" "${37}"
  return 0
}
cpus_of() { grep '^Cpus_allowed_list:' "$1/status" 2>/dev/null | sed 's/^Cpus_allowed_list:[[:space:]]*//'; }

section 'Kernel'
printf 'release: %s\n' "$(value "$R/proc/sys/kernel/osrelease")"
printf 'real-time kernel (/sys/kernel/realtime): %s\n' "$(value "$R/sys/kernel/realtime")"
printf 'command line, CPU and interrupt settings only:'
for t in $(cat "$R/proc/cmdline" 2>/dev/null); do
  case "$t" in
    isolcpus=*|nohz_full=*|rcu_nocbs=*|irqaffinity=*|threadirqs|usbcore.*|usb-storage.*|scsi_mod.*) printf ' %s' "$t" ;;
  esac
done
printf '\n'

section 'Partition 4 (/run/mpfree-data)'
dev=""
while read -r d m t o rest; do
  [ "$m" = /run/mpfree-data ] && { dev=$d; printf 'mounted: %s type %s (%s)\n' "$d" "$t" "$o"; }
done < "$R/proc/mounts"
if [ -z "$dev" ]; then
  printf 'not mounted: the pad player saves nothing this run\n'
else
  df -k "$R/run/mpfree-data" 2>/dev/null | tail -n 1 | while read -r _fs size used avail pct _rest; do
    printf 'size %s KiB, used %s KiB, free %s KiB (%s)\n' "$size" "$used" "$avail" "$pct"
  done
  if [ -d "$R/run/mpfree-data/library" ]; then
    printf 'library folder: present (the read-only disk test reads the audio in it)\n'
  else
    printf 'library folder: none (the read-only disk test needs audio in it)\n'
  fi
  [ -e "$R/run/mpfree-data/mpfree-disktest" ] &&
    printf 'a folder left by an earlier writing disk test: mpfree-disktest (the next writing run deletes it)\n'
fi
part=${dev#/dev/}
disk=""
if [ -n "$part" ] && [ -e "$R/sys/class/block/$part" ]; then
  printf 'partition: %s, number %s, start %s, %s sectors\n' "$part" "$(value "$R/sys/class/block/$part/partition")" \
    "$(value "$R/sys/class/block/$part/start")" "$(value "$R/sys/class/block/$part/size")"
  p=$(readlink -f "$R/sys/class/block/$part" 2>/dev/null)
  disk=$(basename "${p%/*}")
  [ -d "$R/sys/block/$disk" ] || disk=""
fi

section 'The stick (block device and queue)'
if [ -z "$disk" ]; then
  printf 'no disk found behind partition 4\n'
else
  b="$R/sys/block/$disk"
  printf 'disk: %s (%s), %s sectors, removable %s, read-only %s\n' "$disk" "$(value "$b/dev")" \
    "$(value "$b/size")" "$(value "$b/removable")" "$(value "$b/ro")"
  files "$b/device" vendor model rev queue_depth timeout
  printf 'queue:\n'
  files "$b/queue" scheduler nr_requests max_sectors_kb max_hw_sectors_kb max_segments read_ahead_kb rotational \
    write_cache fua logical_block_size physical_block_size minimum_io_size optimal_io_size discard_max_bytes \
    wbt_lat_usec io_poll nomerges rq_affinity
  printf 'I/O counters (stat: reads, merged, sectors, ms, writes, merged, sectors, ms, in flight, ms busy, ...):\n'
  printf '  %s: %s\n' "$disk" "$(value "$b/stat" | tr -s ' ' | sed 's/^ //')"
  printf '  %s: %s\n' "$part" "$(value "$R/sys/class/block/$part/stat" | tr -s ' ' | sed 's/^ //')"
fi

section 'USB path to the stick'
usbdev=""
if [ -n "$disk" ]; then
  d=$(readlink -f "$R/sys/block/$disk" 2>/dev/null)
  while [ -n "$d" ] && [ "$d" != / ] && [ "$d" != "$R" ]; do
    [ -r "$d/idVendor" ] && { usbdev=$d; break; }
    d=${d%/*}
  done
fi
if [ -z "$usbdev" ]; then
  printf 'not on USB, or its USB device was not found\n'
else
  printf 'USB device: %s\n' "$(basename "$usbdev")"
  files "$usbdev" idVendor idProduct manufacturer product version speed bcdDevice bMaxPower maxchild
  for i in "$usbdev"/*:*; do
    [ -d "$i" ] || continue
    drv=none
    [ -L "$i/driver" ] && drv=$(basename "$(readlink "$i/driver")")
    printf '  interface %s: class %s, subclass %s, protocol %s, driver %s\n' "$(basename "$i")" \
      "$(value "$i/bInterfaceClass")" "$(value "$i/bInterfaceSubClass")" "$(value "$i/bInterfaceProtocol")" "$drv"
  done
  h=$usbdev
  while [ -n "$h" ] && [ "$h" != / ]; do
    case "$(basename "$h")" in usb[0-9]*) break ;; esac
    h=${h%/*}
  done
  case "$(basename "$h")" in
    usb[0-9]*)
      c=${h%/*}
      drv=none
      [ -L "$c/driver" ] && drv=$(basename "$(readlink "$c/driver")")
      printf 'root hub: %s, speed %s, version %s; host controller: %s, driver %s\n' "$(basename "$h")" \
        "$(value "$h/speed")" "$(value "$h/version")" "$(basename "$c")" "$drv" ;;
    *) printf 'root hub: not found\n' ;;
  esac
fi

section 'Page cache and writeback'
files "$R/proc/sys/vm" dirty_ratio dirty_background_ratio dirty_bytes dirty_background_bytes dirty_expire_centisecs \
  dirty_writeback_centisecs vfs_cache_pressure min_free_kbytes swappiness page-cluster overcommit_memory
if [ -n "$disk" ]; then
  bdi="$R/sys/class/bdi/$(value "$R/sys/block/$disk/dev")"
  printf 'for the stick (%s):\n' "${bdi#"$R"}"
  files "$bdi" read_ahead_kb min_ratio max_ratio min_ratio_fine max_ratio_fine min_bytes max_bytes strict_limit \
    stable_pages_required
fi

section 'Memory'
grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|Active\(file\)|Inactive\(file\)|Dirty|Writeback|Mlocked|Unevictable|SwapTotal|CmaTotal|CmaFree):' \
  "$R/proc/meminfo" 2>/dev/null | tr -s ' ' | sed 's/^/  /'

section 'USB interrupts and their threads'
grep -i -E 'ehci|ohci|dwc|xhci|usb' "$R/proc/interrupts" 2>/dev/null | while read -r line; do
  n=${line%%:*}
  n=$(printf '%s' "$n" | tr -d ' ')
  printf '%s\n' "$line" | tr -s ' ' | sed 's/^/  /'
  printf '    IRQ %s: CPUs %s (effective %s)\n' "$n" "$(value "$R/proc/irq/$n/smp_affinity_list")" \
    "$(value "$R/proc/irq/$n/effective_affinity_list")"
  for c in "$R"/proc/[0-9]*/comm; do
    case "$(value "$c")" in
      "irq/$n-"*) t=${c%/comm}; printf '    thread %s: %s, CPUs %s\n' "$(value "$c")" "$(sched "$t")" "$(cpus_of "$t")" ;;
    esac
  done
done
for c in "$R"/proc/[0-9]*/comm; do
  case "$(value "$c")" in
    usb-storage|uas*|scsi_eh_*)
      t=${c%/comm}; printf '  %s: %s, CPUs %s\n' "$(value "$c")" "$(sched "$t")" "$(cpus_of "$t")" ;;
  esac
done

section "The pad player's real-time threads"
found=no
for c in "$R"/proc/[0-9]*/comm; do
  [ "$(value "$c")" = mpfree ] || continue
  p=${c%/comm}
  found=yes
  printf 'pad player: pid %s\n' "$(basename "$p")"
  n=0
  for t in "$p"/task/*; do
    [ -r "$t/stat" ] || continue
    s=$(sched "$t")
    case "$s" in
      "policy FIFO"*|"policy RR"*|"policy deadline"*)
        printf '  %s: %s, CPUs %s\n' "$(value "$t/comm")" "$s" "$(cpus_of "$t")"; n=$((n + 1)) ;;
    esac
  done
  [ "$n" = 0 ] && printf '  no real-time threads\n'
done
[ "$found" = yes ] || printf 'the pad player is not running\n'

section 'CPUs'
printf 'online: %s, isolated: %s\n' "$(value "$R/sys/devices/system/cpu/online")" \
  "$(value "$R/sys/devices/system/cpu/isolated")"
for c in "$R"/sys/devices/system/cpu/cpu[0-9]*; do
  [ -d "$c/cpufreq" ] || continue
  printf '  %s: governor %s, now %s kHz, range %s-%s kHz\n' "$(basename "$c")" \
    "$(value "$c/cpufreq/scaling_governor")" "$(value "$c/cpufreq/scaling_cur_freq")" \
    "$(value "$c/cpufreq/scaling_min_freq")" "$(value "$c/cpufreq/scaling_max_freq")"
done

section 'File systems and disks (for the SD card plan)'
printf 'file systems the kernel reads now:'
for fs in vfat msdos exfat ntfs3 ext4 f2fs fuseblk; do
  grep -q "[[:space:]]$fs\$" "$R/proc/filesystems" 2>/dev/null && printf ' %s' "$fs"
done
printf '\n'
printf 'disk modules loaded:'
for m in $(cut -d ' ' -f 1 "$R/proc/modules" 2>/dev/null); do
  case "$m" in exfat|vfat|fat|msdos|ntfs3|dw_mmc*|mmc_block|sdhci*|uas|usb_storage) printf ' %s' "$m" ;; esac
done
printf '\n'
printf 'block devices:'
for b in "$R"/sys/block/*; do [ -e "$b" ] && printf ' %s' "$(basename "$b")"; done
printf '\n'
printf 'mounted from a disk (/proc/mounts):\n'
grep '^/dev/' "$R/proc/mounts" 2>/dev/null | redact | sed 's/^/  /'
s=no
for c in "$R"/proc/[0-9]*/comm; do [ "$(value "$c")" = edisksd ] && s=yes; done
printf "Akai's disk service (edisksd): running %s" "$s"
for u in "$R"/etc/systemd/system/edisksd.service "$R"/lib/systemd/system/edisksd.service \
         "$R"/usr/lib/systemd/system/edisksd.service; do
  if [ -L "$u" ] && [ "$(readlink "$u")" = /dev/null ]; then
    printf ', masked (%s)' "${u#"$R"}"
  elif [ -e "$u" ]; then
    printf ', unit %s' "${u#"$R"}"
  fi
done
printf '\n'

section 'What the kernel said about the stick (this boot)'
pattern='usb [0-9]+-[0-9.]+:|usb-storage|uas|scsi|I/O error|blk_update_request|EXT4-fs|over-current'
[ -n "$disk" ] && pattern="$pattern|$disk"
kernel_log | grep -i -E "$pattern" | tail -n 60 | redact

printf '\nEND OF STREAM SURVEY\n'
