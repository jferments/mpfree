#!/bin/sh
# Third-stage MPFree MPC One read-only audio/runtime probe, POSIX shell.
# Does not open ALSA devices, send MIDI, modify hardware/firmware, stop vendor
# services, or write anything to the MPC.
# Streamed over strictly host-key-checked SSH stdin; output saved on the host.
set -u

section() { printf '\n\n========== %s ==========\n' "$1"; shift; "$@" 2>&1 || echo '[some optional reads unavailable]'; }
read_file() {
  if [ -r "$1" ]; then
    printf '%s:\n' "$1"
    cat "$1" 2>/dev/null || :
  else
    printf 'unavailable (not necessarily unsupported): %s\n' "$1"
  fi
}
current_vendor_pid() {
  command -v systemctl >/dev/null 2>&1 || return 0
  systemctl show acvs.service -p MainPID 2>/dev/null |
    sed -n 's/^MainPID=//p' | head -n 1
}
app_runtime() {
  pid="$(current_vendor_pid)"
  case "$pid" in ''|0|*[!0-9]*) echo 'Unable to identify running acvs MainPID'; return 0;; esac
  [ -d "/proc/$pid" ] || { echo 'acvs PID exited'; return 0; }
  printf 'Factory acvs.service MainPID (volatile): %s\n' "$pid"
  printf 'Factory process executable symlink: '
  readlink "/proc/$pid/exe" 2>/dev/null || echo '[unavailable]'
  if [ -r "/proc/$pid/status" ]; then
    echo 'Selected /proc/pid/status fields:'
    grep -E '^(Name|State|Pid|PPid|Threads|VmRSS|VmSize|VmLck|VmHWM|Cpus_allowed_list):' \
      "/proc/$pid/status" 2>/dev/null || :
  fi
  # Do not traverse smaps_rollup while live audio runs: scanning many VMAs
  # may temporarily consume CPU. /proc/<pid>/status already reports VmLck.
  echo 'Current selected service properties (read-only):'
  systemctl show acvs.service --no-pager -p Type -p Restart -p MainPID \
    -p Requires -p PartOf -p BindsTo -p Wants 2>/dev/null || :
}
app_threads() {
  pid="$(current_vendor_pid)"
  case "$pid" in ''|0|*[!0-9]*) echo 'No active MainPID'; return 0;; esac
  n=0
  for path in /proc/"$pid"/task/[0-9]*; do
    [ -r "$path/comm" ] || continue
    n=$((n+1))
    [ "$n" -le 160 ] || { echo 'Thread list truncated after 160 entries'; break; }
    tid="$(basename "$path")"
    printf 'tid=%s comm=' "$tid"
    cat "$path/comm" 2>/dev/null | tr '\n' ' '
    printf '\n'
    if [ -r "$path/sched" ]; then
      grep -E '^[[:space:]]*(policy|prio|nr_switches)[[:space:]]*:' "$path/sched" 2>/dev/null || :
    fi
  done
}
alsa_active_procfs() {
  for direction in p c; do
    for name in info hw_params sw_params status; do
      path="/proc/asound/card1/pcm0$direction/sub0/$name"
      read_file "$path"
    done
  done
  for path in /proc/asound/card0/midi0 \
              /proc/asound/card0/usbmidi \
              /proc/asound/seq/clients; do
    if [ -r "$path" ]; then
      printf '%s (up to 90 lines):\n' "$path"
      head -n 90 "$path" 2>/dev/null || :
    fi
  done
}
usb_midi_interfaces() {
  for path in /sys/bus/usb/devices/1-1:1.*; do
    [ -d "$path" ] || continue
    printf 'Device interface: %s\n' "$(basename "$path")"
    for name in bInterfaceNumber bInterfaceClass bInterfaceSubClass \
                bInterfaceProtocol bNumEndpoints; do
      file="$path/$name"
      if [ -r "$file" ]; then
        printf '  %s=' "$name"
        cat "$file" 2>/dev/null || :
      fi
    done
    printf '  bound_driver='
    readlink "$path/driver" 2>/dev/null || echo '[not reported]'
  done
}
elf_header() {
  # Observation of ELF header only, NOT an executable copy.
  if [ -r /usr/bin/MPC ] && command -v dd >/dev/null 2>&1 &&
     command -v od >/dev/null 2>&1; then
    echo 'Factory /usr/bin/MPC ELF header, first 64 bytes only, as octal:'
    dd if=/usr/bin/MPC bs=64 count=1 2>/dev/null | od -b
  else
    echo 'ELF header probe unavailable'
  fi
  if [ -r /lib/ld-linux-armhf.so.3 ]; then
    echo 'Installed: /lib/ld-linux-armhf.so.3'
  fi
}

printf 'MPFREE MPC ONE READ-ONLY AUDIO/RUNTIME SURVEY\n'
section 'Stock MPC process and memory summary' app_runtime
section 'Stock MPC thread policies (read-only sched metadata)' app_threads
section 'Current ACVA ALSA PCM kernel procfs parameters' alsa_active_procfs
section 'Internal USB MIDI interface descriptors' usb_midi_interfaces
section 'Factory ARM ELF header and installed loader' elf_header
printf '\nEND OF AUDIO/RUNTIME SURVEY\n'
