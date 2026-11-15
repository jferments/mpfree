#!/bin/sh
# MPFree kexec preflight - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --kexec-preflight.
#
# Answers, before any kexec test: can the running 3.6 system load another
# kernel, how much memory and which address ranges does it have, can the other
# CPU cores be taken offline, and what does the running device tree hold
# (property names and sizes only; the workstation copies the binary tree and
# compares it with Akai's files there).
#
# It reads procfs, sysfs and /proc/device-tree, and asks the kexec and
# systemctl programs for their version and help text. It NEVER loads or runs a
# kernel (kexec is only called with --version and --help), never mounts,
# never starts, stops or changes a service, and never writes storage.
set -u
printf 'MPFREE KEXEC PREFLIGHT - READ ONLY\n'
printf 'Values that identify the unit (serial, MAC, partition UUIDs) are left out.\n'

section() { printf '\n=== %s ===\n' "$1"; }
show() { [ -r "$1" ] && printf '%s: %s\n' "$1" "$(cat "$1")"; return 0; }
dt_text() { [ -r "$1" ] && tr '\000' ' ' < "$1"; return 0; }

section 'Kernel and command line'
uname -r
cat /proc/version 2>/dev/null || :
sed -e 's/PARTUUID=[^ ]*/PARTUUID=<REDACTED>/g' \
    -e 's/serialno=[^ ]*/serialno=<REDACTED>/g' -e 's/ethaddr=[^ ]*/ethaddr=<REDACTED>/g' \
    /proc/cmdline 2>/dev/null || :

section 'kexec program'
if command -v kexec >/dev/null 2>&1; then
  printf 'path: %s\n' "$(command -v kexec)"
  kexec --version 2>&1 | head -n 3
  printf 'options from --help that a kexec boot would use:\n'
  kexec --help 2>&1 | grep -E -e '--(dtb|command-line|append|initrd|ramdisk|load|exec|type|reuse-cmdline|kexec-syscall|kexec-file-syscall|kexec-syscall-auto|status|mem-min|mem-max|atags|image-size|page-offset)' | head -n 40
else
  printf 'kexec: not found\n'
fi

section 'kexec state in the kernel'
for f in /sys/kernel/kexec_loaded /sys/kernel/kexec_crash_loaded /sys/kernel/kexec_crash_size \
         /proc/sys/kernel/kexec_load_disabled /proc/sys/kernel/kexec_load_limit_reboot \
         /proc/sys/kernel/kexec_load_limit_panic; do
  show "$f"
done

section 'Kernel config symbols that matter for kexec'
if [ -r /proc/config.gz ] && command -v zcat >/dev/null 2>&1; then
  zcat /proc/config.gz | grep -E '^(# )?CONFIG_(KEXEC[A-Z_]*|SMP|NR_CPUS|HOTPLUG_CPU|HIGHMEM|VMSPLIT_[0-9A-Z_]+|ARM_ATAG_DTB_COMPAT|ARM_APPENDED_DTB|CMDLINE[A-Z_]*|SECURITY_LOADPIN[A-Z_]*|SECURITY_LOCKDOWN[A-Z_]*|SECURITY_IPE[A-Z_]*|LSM|MODULE_SIG[A-Z_]*|STRICT_DEVMEM|IO_STRICT_DEVMEM|PROC_SYSCTL|BLK_DEV_INITRD|RD_[A-Z0-9]+|USB_STORAGE|BLK_DEV_SD|EXT4_FS)(=| is not set)' || :
else
  printf '/proc/config.gz unavailable\n'
fi

section 'systemd (a clean kexec boot is started through systemd)'
systemctl --version 2>/dev/null | head -n 1
systemctl show -p Id -p LoadState -p UnitFileState -p ActiveState kexec.target 2>/dev/null || :
systemctl show -p Id -p LoadState -p ActiveState systemd-kexec.service 2>/dev/null || :
printf 'system state: %s\n' "$(systemctl is-system-running 2>/dev/null)"

section 'Memory'
grep -E '^(MemTotal|MemFree|MemAvailable|HighTotal|HighFree|LowTotal|LowFree|CmaTotal|CmaFree|Shmem):' /proc/meminfo 2>/dev/null || :

section 'Physical memory map (System RAM, kernel and reserved ranges from /proc/iomem)'
grep -E 'System RAM|Kernel|reserved|Crash' /proc/iomem 2>/dev/null || :

section 'CPUs (kexec takes the other cores offline first)'
show /sys/devices/system/cpu/possible
show /sys/devices/system/cpu/online
for c in /sys/devices/system/cpu/cpu[0-9]*; do
  [ -d "$c" ] || continue
  if [ -e "$c/online" ]; then printf '%s: hotplug control present\n' "${c##*/}"
  else printf '%s: no hotplug control\n' "${c##*/}"; fi
done

section 'Running device tree (names and sizes only)'
ls -l /sys/firmware/fdt 2>/dev/null || printf '/sys/firmware/fdt: not readable\n'
printf 'model: '; dt_text /proc/device-tree/model; printf '\n'
printf 'compatible: '; dt_text /proc/device-tree/compatible; printf '\n'
printf 'root properties:'
for f in /proc/device-tree/*; do [ -f "$f" ] && printf ' %s(%s)' "${f##*/}" "$(wc -c < "$f")"; done
printf '\n'
printf '/chosen properties:'
for f in /proc/device-tree/chosen/*; do [ -f "$f" ] && printf ' %s(%s)' "${f##*/}" "$(wc -c < "$f")"; done
printf '\n'
for n in /proc/device-tree/*@ff0f0000 /proc/device-tree/*@ff0c0000; do
  [ -d "$n" ] || continue
  printf '%s: status ' "${n##*/}"
  if [ -r "$n/status" ]; then dt_text "$n/status"; else printf '(none: okay)'; fi
  printf '\n'
done
for n in /proc/device-tree/ethernet@*; do
  [ -d "$n" ] || continue
  printf '%s:' "${n##*/}"
  for p in local-mac-address mac-address; do [ -f "$n/$p" ] && printf ' has %s' "$p"; done
  printf '\n'
done
printf 'sound node: '; if [ -d /proc/device-tree/sound ]; then printf 'present\n'; else printf 'absent\n'; fi

section 'eMMC write counters (field 7 = sectors written since boot)'
for dev in /sys/class/block/mmcblk0 /sys/class/block/mmcblk0p* \
           /sys/class/block/mmcblk0boot0 /sys/class/block/mmcblk0boot1; do
  [ -r "$dev/stat" ] && printf '%s %s\n' "${dev##*/}" "$(cat "$dev/stat")"
done

section 'RAM folders and removable storage'
grep -E ' (/run|/tmp|/dev/shm|/media[^ ]*|/mnt[^ ]*) ' /proc/mounts 2>/dev/null || :
df -k /run /tmp /dev/shm 2>/dev/null || :
for d in /sys/block/sd*; do
  [ -d "$d" ] || continue
  printf '%s: removable=%s size_sectors=%s\n' "${d##*/}" "$(cat "$d/removable" 2>/dev/null)" "$(cat "$d/size" 2>/dev/null)"
done

section 'LoadPin'
if [ -d /sys/kernel/security/loadpin ]; then ls /sys/kernel/security/loadpin; else printf 'no /sys/kernel/security/loadpin\n'; fi
# The kernel's sysctl is kernel.loadpin.enforce (security/loadpin/loadpin.c). With enforce=1 LoadPin refuses
# every raw kexec_load ("kexec-image old-api-denied"), whatever IPE says, and on a read-only (dm-verity)
# pinned root the sysctl cannot be lowered at runtime; only loadpin.enforce=0 or loadpin.exclude=kexec-image
# on the kernel command line lift it. kexec_file_load is allowed only from the pinned or a trusted verity
# filesystem. The older name (enabled) is read too, in case a vendor kernel used it.
show /proc/sys/kernel/loadpin/enforce
show /proc/sys/kernel/loadpin/enabled
printf 'command-line words for LoadPin, IPE, lockdown and the LSM list: %s\n' \
  "$(tr ' ' '\n' < /proc/cmdline 2>/dev/null | grep -E '^(loadpin|ipe|lockdown|lsm|security)[.=]' | tr '\n' ' ')"

# What can refuse loading a kernel at runtime (so the owner knows before any install whether an unsigned
# kexec will be accepted). A kexec_load that returns "Operation not permitted" with kexec_load_disabled=0
# and root means one of these refused it, not the tool. Reads only: small sysfs files and the kernel log.
section 'Kexec gate (what can refuse loading a kernel)'
show /sys/kernel/security/lockdown
if [ -d /sys/kernel/security/ipe ]; then
  printf 'IPE (Integrity Policy Enforcement) is present\n'
  show /sys/kernel/security/ipe/active_policy
  show /sys/kernel/security/ipe/enforce
  if [ -d /sys/kernel/security/ipe/policies ]; then
    for p in /sys/kernel/security/ipe/policies/*; do
      [ -d "$p" ] || continue
      act=$(cat "$p/active" 2>/dev/null)
      printf 'policy %s: active=%s version=%s\n' "${p##*/}" "$act" "$(cat "$p/version" 2>/dev/null)"
      # The active policy's rules say which sources (dm-verity, fs-verity, a signed boot kernel) each
      # kernel operation trusts. Reading it says whether a signed kexec from a file could be accepted where
      # the raw-buffer load is refused. Rules only; no values that identify the unit. Bounded read.
      if [ "$act" = 1 ] && [ -r "$p/policy" ]; then
        printf -- '--- active IPE policy (first 8 KiB; KEXEC lines matter) ---\n'
        dd if="$p/policy" bs=8192 count=1 2>/dev/null | tr -cd '\11\12\15\40-\176'
        printf -- '\n--- end policy ---\n'
      fi
    done
  fi
else
  printf 'no /sys/kernel/security/ipe\n'
fi
printf 'kernel log, kexec/lockdown/ipe/loadpin lines:\n'
dmesg 2>/dev/null | grep -iE 'ipe:|lockdown|kexec|loadpin' | tail -n 30 || printf '(none)\n'

# A route that loads no kernel: systemd's soft-reboot (systemd 254 and later) stops all of userspace and,
# when /run/nextroot holds an OS tree, starts that tree's init on the same running kernel. Whether this
# systemd has it, and whether /run/nextroot exists now (it should not). Asks for help text only.
section 'Userspace restart without a new kernel (soft-reboot)'
if systemctl --help 2>/dev/null | grep -q 'soft-reboot'; then printf 'systemctl knows soft-reboot\n'
else printf 'systemctl does not list soft-reboot\n'; fi
[ -e /usr/lib/systemd/system/soft-reboot.target ] && printf 'soft-reboot.target: present\n' || printf 'soft-reboot.target: absent\n'
[ -e /usr/lib/systemd/system/systemd-soft-reboot.service ] && printf 'systemd-soft-reboot.service: present\n' || printf 'systemd-soft-reboot.service: absent\n'
[ -e /run/nextroot ] && printf '/run/nextroot: exists\n' || printf '/run/nextroot: absent\n'

printf '\nEND OF KEXEC PREFLIGHT\n'
