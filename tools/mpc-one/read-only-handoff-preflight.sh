#!/bin/sh
# Strictly non-invasive research about future stock-app handoff/recovery.
# NEVER stop, restart, mask, kill, write, bind, umount or change any devices.
set -u
printf 'MPFREE STOCK APP REVERSIBILITY PRE-FLIGHT - READ ONLY\n'
printf 'This is NOT a reversible-handoff implementation or recovery validation.\n'
printf '\n=== Current systemd service units and restart rules ===\n'
for name in acvs.service az01-network-midi.service az01-script-runner.service; do
  echo "--- $name ---"
  if command -v systemctl >/dev/null 2>&1; then
    systemctl show "$name" --no-pager \
      -p Id -p LoadState -p ActiveState -p SubState -p Type \
      -p FragmentPath -p DropInPaths -p MainPID -p ExecMainPID \
      -p ExecStart -p ExecStop -p ExecStartPre -p ExecStopPost \
      -p Restart -p RestartUSec -p TimeoutStopUSec -p KillMode \
      -p Requires -p Wants -p After -p Before -p PartOf -p Conflicts \
      -p BindsTo -p WantedBy -p RequiredBy 2>&1 || :
    echo '--- Unit file body (observe only; review before sharing) ---'
    systemctl cat --no-pager "$name" 2>&1 | head -n 160 || :
  else
    echo 'systemctl unavailable'
  fi
done
printf '\n=== Factory executable and currently owned resources ===\n'
if command -v systemctl >/dev/null 2>&1; then
  pid="$(systemctl show acvs.service -p MainPID 2>/dev/null | sed -n 's/^MainPID=//p')"
else pid=''; fi
case "$pid" in ''|0|*[!0-9]*) echo 'No stable active acvs MainPID; skip fd scan';;
*)
  if [ -d "/proc/$pid" ]; then
    printf 'MainPID (transient, do not hardcode): %s\n' "$pid"
    printf 'Executable: '; readlink "/proc/$pid/exe" 2>/dev/null || :
    if [ -r "/proc/$pid/status" ]; then
      grep -E '^(Name|State|Threads|VmRSS|VmLck|Cpus_allowed_list):' \
        "/proc/$pid/status" 2>/dev/null || :
    fi
    n=0
    for fd in /proc/"$pid"/fd/*; do
      [ -L "$fd" ] || continue
      n=$((n+1))
      [ "$n" -le 256 ] || { echo 'FD list truncated to 256'; break; }
      target="$(readlink "$fd" 2>/dev/null || :)"
      case "$target" in
        /dev/snd/*|/dev/dri/*|/dev/input/*|/dev/midi*|/dev/tty*|/dev/gpio*)
          printf 'fd %s -> %s\n' "$(basename "$fd")" "$target"
          ;;
      esac
    done
  fi
  ;;
esac
printf '\n=== ALSA PCM subdevice availability ===\n'
for direction in p c; do
  file="/proc/asound/card1/pcm0$direction/info"
  if [ -r "$file" ]; then
    echo "--- $file ---"
    cat "$file" 2>/dev/null || :
  fi
done
printf '\n=== Display and input device node names (no opens) ===\n'
for path in /dev/dri/card* /dev/dri/render* /dev/input/event*; do
  [ -e "$path" ] && ls -ld "$path" 2>/dev/null || :
done
printf '\n=== Root and persistent overlays (no mounts changed) ===\n'
if [ -r /proc/mounts ]; then
  awk '$2=="/" || $2=="/etc" || $2=="/var" || $2=="/run" || $2=="/tmp" || $2=="/data" {print}' /proc/mounts
fi
printf '\n=== RAM and uptime snapshot ===\n'
if [ -r /proc/meminfo ]; then
  grep -E '^(MemTotal|MemAvailable|Mlocked|SwapTotal):' /proc/meminfo || :
fi
if [ -r /proc/uptime ]; then cat /proc/uptime; fi
printf '\n=== Boot command line (may contain identifiers; do not publish raw) ===\n'
if [ -r /proc/cmdline ]; then cat /proc/cmdline; fi
printf '\n=== Boot partitions (names only; never open raw block devices) ===\n'
if [ -r /proc/partitions ]; then cat /proc/partitions; fi
printf '\nEND MPFREE REVERSIBLE-HANDOFF PRE-FLIGHT\n'
printf 'BLOCKED: no verified independent recovery or stock-state restore test.\n'
