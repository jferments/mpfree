#!/bin/sh
# Read-only two-snapshot CPU and memory resource accounting for factory MPC.
# Runs on the already-authenticated MPC through strict-identity-gated survey.
# Does NOT open ALSA/MIDI/DRM, change affinity, send signals, touch overlays,
# or write to devices or disk. Executes ~2 seconds of ordinary passive reads.
set -u
printf 'MPFREE STOCK RESOURCE BUDGET SURVEY - READ ONLY\n'
printf 'This is a brief snapshot, NOT a realtime performance benchmark.\n'
if ! command -v systemctl >/dev/null 2>&1; then
  echo 'systemctl unavailable; no process data'; exit 0
fi
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)"
case "$pid" in
  ''|0|*[!0-9]*) echo 'No valid active factory MainPID'; exit 0 ;;
esac
if [ ! -r "/proc/$pid/status" ]; then
  echo 'Factory process disappeared before observation'; exit 0
fi
printf '\n=== Kernel CPU topology ===\n'
for file in /sys/devices/system/cpu/online /sys/devices/system/cpu/isolated; do
  if [ -r "$file" ]; then printf '%s: ' "$file"; cat "$file"; fi
done
printf '\n=== Total physical and reclaimable memory (KiB) ===\n'
if [ -r /proc/meminfo ]; then
  grep -E '^(MemTotal|MemFree|MemAvailable|Cached|Buffers|Shmem|KReclaimable|SReclaimable|Slab|Mlocked|Unevictable|SwapTotal|SwapFree):' /proc/meminfo || :
fi
printf '\n=== Factory process accounting (KiB; NOT per-thread RSS) ===\n'
grep -E '^(Name|State|Threads|VmRSS|VmLck|VmPin|VmSize|RssAnon|RssFile|RssShmem|VmSwap):' "/proc/$pid/status" 2>/dev/null || :
if [ -r "/proc/$pid/smaps_rollup" ]; then
  printf '\n=== Process proportional/private memory from smaps_rollup (single read) ===\n'
  # One procfs summary, not every mapping. May require root and kernel support.
  grep -E '^(Rss|Pss|Pss_Anon|Pss_File|Pss_Shmem|Shared_Clean|Shared_Dirty|Private_Clean|Private_Dirty|Anonymous|Locked|Swap):' "/proc/$pid/smaps_rollup" 2>/dev/null || :
else
  echo 'smaps_rollup unavailable; cannot quantify proportional/private RSS'
fi
printf '\n=== Memory/CPU pressure counters (read only) ===\n'
for file in /proc/pressure/cpu /proc/pressure/memory /proc/pressure/io; do
  if [ -r "$file" ]; then
    printf '%s\n' "$file"
    cat "$file" 2>/dev/null || :
  fi
done

# /proc/PID/task/TID/stat is documented Linux procfs data:
# after removing "pid (comm) ", fields become state=$1, utime=$12,
# stime=$13, last_processor=$37, rt_priority=$38, policy=$39.
# The comm field can contain spaces, so NEVER split it before stripping.
app_ticks() {
  awk '{
    sub(/^.*\) /, "")
    if (NF>=39) {ticks += $12 + $13; tasks++}
  } END {printf "factory_thread_ticks=%.0f sampled_threads=%d\n", ticks, tasks}' \
    /proc/"$pid"/task/[0-9]*/stat 2>/dev/null || :
}
core_ticks() {
  awk '$1 ~ /^cpu[0-9]+$/ {
    total=0;
    for (i=2; i<=9 && i<=NF; i++) total += $i;
    printf "%s total_ticks=%.0f idle_iowait_ticks=%.0f\n", $1, total, $5 + $6
  }' /proc/stat 2>/dev/null || :
}
printf '\n=== CPU counters snapshot A (system-wide + ALL factory threads) ===\n'
core_ticks
app_ticks
printf '\n=== Two-second sampling interval (no device operations) ===\n'
sleep 2
if [ ! -r "/proc/$pid/status" ]; then
  echo 'Factory process vanished during interval; second snapshot invalid'
  exit 0
fi
printf '\n=== CPU counters snapshot B ===\n'
core_ticks
app_ticks
printf '\nEND MPFREE STOCK RESOURCE BUDGET SURVEY\n'
printf 'Interpret counter deltas on the host; a pinned thread is NOT a busy core.\n'
