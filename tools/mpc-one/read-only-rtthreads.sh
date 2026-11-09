#!/bin/sh
# Read-only per-thread scheduler/CPU-affinity survey of factory MPC.
# No /proc/<pid>/mem, no fd opens, no task manipulation.
set -u
printf 'MPFREE FACTORY THREAD SCHEDULING SURVEY - READ ONLY\n'
if ! command -v systemctl >/dev/null 2>&1; then
  echo 'systemctl unavailable'; exit 0
fi
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)"
case "$pid" in
  ''|0|*[!0-9]*) echo 'No active acvs MainPID; skip task enumeration'; exit 0 ;;
esac
if [ ! -d "/proc/$pid/task" ]; then
  echo 'Factory task directory disappeared; no data'; exit 0
fi
printf 'Factory MainPID: %s (transient)\n' "$pid"
printf '=== CPU topology ===\n'
for f in /sys/devices/system/cpu/online /sys/devices/system/cpu/isolated; do
  if [ -r "$f" ]; then printf '%s: ' "$f"; cat "$f"; fi
done
printf '\n=== systemd scheduling hints for acvs.service ===\n'
systemctl show acvs.service --no-pager -p CPUAffinity -p CPUWeight \
  -p Nice -p CPUSchedulingPolicy -p CPUSchedulingPriority -p MemoryMax \
  -p LimitMEMLOCK -p LimitRTPRIO -p LimitRTTIME 2>&1 || :
printf '\n=== Task name, state, CPU affinity, scheduling policy/priority ===\n'
count=0
for tid_dir in /proc/"$pid"/task/[0-9]*; do
  [ -d "$tid_dir" ] || continue
  count=$((count+1))
  if [ "$count" -gt 128 ]; then
    printf 'Task list truncated at 128\n'; break
  fi
  tid="$(basename "$tid_dir")"
  printf '\n--- TID %s ---\n' "$tid"
  if [ -r "$tid_dir/status" ]; then
    grep -E '^(Name|State|Cpus_allowed_list|Mems_allowed_list|VmRSS):' \
      "$tid_dir/status" 2>/dev/null || :
  fi
  if [ -r "$tid_dir/sched" ]; then
    awk '/^[[:space:]]*(policy|prio|nr_switches|nr_voluntary_switches|nr_involuntary_switches)[[:space:]]*:/ {print}' \
      "$tid_dir/sched" 2>/dev/null || :
  fi
done
printf '\n=== Overall process memory and limits ===\n'
if [ -r "/proc/$pid/status" ]; then
  grep -E '^(VmRSS|VmLck|Threads|Cpus_allowed_list):' "/proc/$pid/status" || :
fi
if [ -r "/proc/$pid/limits" ]; then
  grep -E 'Max (locked memory|realtime priority|realtime timeout)' \
    "/proc/$pid/limits" || :
fi
printf '\nEND MPFREE FACTORY THREAD SCHEDULING SURVEY\n'
