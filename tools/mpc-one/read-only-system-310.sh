#!/bin/sh
# READ-ONLY facts about an installed MPC OS 3.10.1 (or whatever runs on the MPC), for
# `collect-over-ssh.sh --system-survey`. Every line reads files or lists things; nothing is
# written, mounted, started, stopped or loaded. Runs under BusyBox sh. A missing file or tool
# prints a note instead of failing. The CPU serial number is left out of /proc/cpuinfo.
set -u

section() { printf '\n===== %s =====\n' "$1"; }
show() { [ -r "$1" ] && sed 's/^/  /' "$1" 2>/dev/null || printf '  (%s: not readable)\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

section 'Versions'
printf 'MPC OS (/etc/version): '; head -n 3 /etc/version 2>/dev/null || echo '(none)'
printf 'kernel: '; uname -a 2>/dev/null
printf 'command line: '; cat /proc/cmdline 2>/dev/null

section 'CPU: clocks, idle states, cores'
awk '!/^Serial[[:space:]]*:/' /proc/cpuinfo 2>/dev/null | grep -E '^(processor|model name|Hardware|Revision|CPU part|Features)' | sed 's/^/  /'
for c in /sys/devices/system/cpu/cpu[0-9]*; do
  f="$c/cpufreq"
  [ -d "$f" ] || { echo "  ${c##*/}: no cpufreq"; continue; }
  echo "  ${c##*/}: governor $(cat "$f/scaling_governor" 2>/dev/null), now $(cat "$f/scaling_cur_freq" 2>/dev/null) kHz," \
       "max $(cat "$f/cpuinfo_max_freq" 2>/dev/null) kHz, steps: $(cat "$f/scaling_available_frequencies" 2>/dev/null)"
done
echo "  cpuidle driver: $(cat /sys/devices/system/cpu/cpuidle/current_driver 2>/dev/null || echo none)"
echo "  isolated cores: $(cat /sys/devices/system/cpu/isolated 2>/dev/null)"
for z in /sys/class/thermal/thermal_zone*; do
  [ -r "$z/temp" ] && echo "  ${z##*/} ($(cat "$z/type" 2>/dev/null)): $(cat "$z/temp") milli-degrees C"
done

section 'Memory'
head -n 5 /proc/meminfo 2>/dev/null | sed 's/^/  /'

section 'kexec and kernel security (what the MPFree test boot needs)'
if [ -x /usr/sbin/kexec ]; then echo "  /usr/sbin/kexec: present"; else echo "  /usr/sbin/kexec: MISSING"; fi
echo "  kexec_load_disabled: $(cat /proc/sys/kernel/kexec_load_disabled 2>/dev/null)"
echo "  kexec_loaded: $(cat /sys/kernel/kexec_loaded 2>/dev/null)"
echo "  lockdown: $(cat /sys/kernel/security/lockdown 2>/dev/null || echo 'not readable (no lockdown file)')"
echo "  security modules: $(cat /sys/kernel/security/lsm 2>/dev/null)"
echo "  LoadPin enforce: $(cat /proc/sys/kernel/loadpin/enforce 2>/dev/null || echo 'not readable')"
echo "  LoadPin trusted verity digests: $(cat /sys/kernel/security/loadpin/dm-verity 2>/dev/null | wc -l) line(s)"

section 'Network: interfaces and listening ports (developer mode SSH is open to every network the MPC is on)'
for n in /sys/class/net/*; do
  i=${n##*/}
  [ "$i" = lo ] && continue
  a=$(ip -4 -o addr show dev "$i" 2>/dev/null | while read -r x y z addr rest; do printf '%s ' "$addr"; done)
  echo "  $i: $(cat "$n/operstate" 2>/dev/null), IPv4: ${a:-none}"
done
for t in /proc/net/tcp /proc/net/tcp6; do
  [ -r "$t" ] || continue
  while read -r sl local remote st rest; do
    [ "$st" = 0A ] && printf '%d\n' "0x${local##*:}"
  done < "$t"
done | sort -n -u | tr '\n' ' ' | sed 's/^/  listening TCP ports: /'; echo

section 'Sound and MIDI'
show /proc/asound/cards
echo "  ALSA sequencer clients:"
show /proc/asound/seq/clients
if have aconnect; then echo "  aconnect -l:"; aconnect -l 2>&1 | sed 's/^/  /'; else echo "  (no aconnect)"; fi
if have amidi; then echo "  amidi -l:"; amidi -l 2>&1 | sed 's/^/  /'; else echo "  (no amidi)"; fi
echo "  sequencer modules in /lib/modules/$(uname -r)/kernel/sound/core/seq/:"
ls "/lib/modules/$(uname -r)/kernel/sound/core/seq/" 2>&1 | sed 's/^/    /'
echo "  loaded sound modules:"
grep -E '^snd' /proc/modules 2>/dev/null | cut -d' ' -f1 | tr '\n' ' ' | sed 's/^/    /'; echo

section 'Input devices'
show /proc/bus/input/devices
ls -l /dev/input /dev/uinput 2>&1 | sed 's/^/  /'

section 'Drives (mount points and the names at their top level)'
grep -E ' /(media|mnt|run/media)' /proc/mounts 2>/dev/null | sed 's/^/  /'
grep -E ' /(media|mnt|run/media)' /proc/mounts 2>/dev/null | while read -r dev mnt rest; do
  mnt=$(printf '%b' "$mnt")
  echo "  top level of $mnt:"
  ls -1 "$mnt" 2>&1 | head -n 40 | sed 's/^/    /'
done

section 'Screen and GPU devices'
ls -l /dev/fb* /dev/dri 2>&1 | sed 's/^/  /'

section 'Controller scripts (Lua files under controller folders)'
find / -xdev -iname '*.lua' -path '*ontroller*' 2>/dev/null | head -n 40 | sed 's/^/  /'

section "Akai's app"
for p in /proc/[0-9]*; do
  c=$(tr '\0' ' ' < "$p/cmdline" 2>/dev/null)
  case "$c" in */MPC\ *|*/MPC|*az01-launch*) echo "  ${p##*/}: $c ($(ls "$p/task" 2>/dev/null | wc -l) threads)" ;; esac
done
echo
echo "END of read-only system survey"
