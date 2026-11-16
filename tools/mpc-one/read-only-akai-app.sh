#!/bin/sh
# READ-ONLY, on the MPC (MPC OS 3.10.1 developer mode, or any shell on the MPC):
# what Akai's running app uses, has loaded and has open. It reads /proc only and
# changes nothing: no service is stopped, started or signalled, no file written.
#
# From the workstation, over the developer-mode SSH (your own login):
#   ssh <login>@<mpc-address> 'sh -s' < tools/mpc-one/read-only-akai-app.sh > akai-app-empty.txt
# Run it twice with Akai's app idle (transport stopped):
#   1. an empty project            -> akai-app-empty.txt
#   2. Hype, Mellotron and Fabric (whichever you have) loaded on tracks -> akai-app-hype.txt
# The difference shows what the instruments cost and whether they are separate
# library files (they appear in "files it has mapped" only after loading).
#
# Why "transport stopped": the memory summary (smaps_rollup) walks the app's
# page tables, which could cost its audio thread a few milliseconds.
# It never reads the app's environment or the contents of any file it has open:
# only names, sizes, counts and kernel settings.
set -u
APP_EXE=${MPFREE_APP_EXE:-/usr/bin/MPC}
APP_COMM=${MPFREE_APP_COMM:-MPC}

pid='' by_name=''
for p in /proc/[0-9]*; do
  state=$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "$p/status" 2>/dev/null) || state=''
  [ "$state" = Z ] && continue  # a finished process waiting to be reaped
  exe=$(readlink "$p/exe" 2>/dev/null) || exe=''
  if [ "$exe" = "$APP_EXE" ]; then pid=${p#/proc/}; break; fi
  comm=$(cat "$p/comm" 2>/dev/null) || comm=''
  [ -z "$by_name" ] && [ "$comm" = "$APP_COMM" ] && by_name=${p#/proc/}
done
[ -n "$pid" ] || pid=$by_name
echo "MPFREE READ-ONLY: Akai's app as it runs now (changes nothing)"
date -u +%Y-%m-%dT%H:%M:%SZ
cat /etc/version 2>/dev/null || :
uname -r
if [ -z "$pid" ]; then
  echo "STOP: Akai's app ($APP_EXE) is not running"
  exit 1
fi
echo "pid $pid, executable $(readlink "/proc/$pid/exe" 2>/dev/null || echo '(unreadable)')"

echo; echo "=== memory ==="
grep -E '^(VmPeak|VmSize|VmLck|VmPin|VmHWM|VmRSS|RssAnon|RssFile|RssShmem|VmSwap|Threads|Cpus_allowed_list|CapEff):' \
  "/proc/$pid/status" 2>/dev/null || echo '(status unreadable)'
if [ -r "/proc/$pid/smaps_rollup" ]; then cat "/proc/$pid/smaps_rollup"; else echo '(no smaps_rollup)'; fi
grep -E '^(MemTotal|MemFree|MemAvailable|Cached|Mlocked|Unevictable|SwapTotal|CmaTotal|CmaFree):' /proc/meminfo

echo; echo "=== limits and cgroup ==="
grep -E 'locked memory|realtime|nice|open files' "/proc/$pid/limits" 2>/dev/null || echo '(limits unreadable)'
cat "/proc/$pid/cgroup" 2>/dev/null || :

echo; echo "=== CPU ==="
echo "clock ticks per second: $(getconf CLK_TCK 2>/dev/null || echo 100)"
cat /proc/uptime
stat_rest=$(cat "/proc/$pid/stat" 2>/dev/null) || stat_rest=''
stat_rest=${stat_rest##*) }
# shellcheck disable=SC2086
set -- $stat_rest
[ $# -ge 20 ] && echo "app CPU so far: user ${12} ticks, system ${13} ticks; started at tick ${20}"
grep -E '^cpu[0-9 ]' /proc/stat
echo "threads: name | CPU | rt priority | policy (0 normal, 1 FIFO, 2 RR)"
for t in /proc/"$pid"/task/*; do
  name=$(cat "$t/comm" 2>/dev/null) || continue
  s=$(cat "$t/stat" 2>/dev/null) || continue
  s=${s##*) }
  # shellcheck disable=SC2086
  set -- $s
  [ $# -ge 39 ] && echo "  $name | ${37} | ${38} | ${39}"
done

echo; echo "=== files it has mapped (libraries, plugins, data) ==="
awk '$6 ~ /^\// { print $6 }' "/proc/$pid/maps" 2>/dev/null | sort -u

echo; echo "=== files and devices it has open (names only) ==="
for fd in /proc/"$pid"/fd/*; do readlink "$fd" 2>/dev/null; done |
  sed -e 's/^socket:.*/socket/' -e 's/^pipe:.*/pipe/' -e 's/^anon_inode:\(.*\)/anon_inode:\1/' | sort | uniq -c

echo; echo "=== sound, MIDI and input devices ==="
cat /proc/asound/cards 2>/dev/null || :
cat /proc/asound/pcm 2>/dev/null || :
cat /proc/asound/seq/clients 2>/dev/null | grep -E '^Client|Port' || :
grep -E '^N: Name=' /proc/bus/input/devices 2>/dev/null || :

echo; echo "=== kernel features for guest mode ==="
if [ -r /proc/config.gz ]; then
  zcat /proc/config.gz | grep -E '^(# )?CONFIG_(DRM_VKMS|SND_ALOOP|SND_VIRMIDI|SND_SEQ_DUMMY|INPUT_UINPUT|NAMESPACES|USER_NS|PID_NS|NET_NS|IPC_NS|CGROUPS|MEMCG|CPUSETS|CGROUP_SCHED|RT_GROUP_SCHED|MODULES|MODULE_SIG_FORCE|ZRAM|SWAP)(=| is not set)'
else
  echo '(no /proc/config.gz)'
fi
echo "modules loaded now:"
awk '{ print "  " $1 }' /proc/modules 2>/dev/null || :
for f in /proc/sys/kernel/unprivileged_userns_clone /proc/sys/user/max_user_namespaces; do
  [ -r "$f" ] && echo "$f = $(cat "$f")"
done
ls /dev/uinput /dev/dri 2>/dev/null || :

echo; echo "DONE: read-only. Upload this file to the project thread."
