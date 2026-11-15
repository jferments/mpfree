#!/bin/sh
# Read-only survey for the MPC plugin SDK work on MPC OS 3.10.1:
# developer SSH, how Akai's app runs (user, limits, sandboxing, threads),
# whether external plugins are set up, and what a plugin could reach.
# NEVER stops, restarts, kills, writes, mounts, opens devices or loads anything.
# It reads procfs, sysfs, systemd properties and a few config files only.
set -u
printf 'MPFREE PLUGIN HOST SURVEY - READ ONLY\n'
printf 'Review before sharing: the report can hold addresses, unit environments and file names.\n'

printf '\n=== Firmware and kernel ===\n'
for f in /etc/version /etc/timestamp /proc/device-tree/model; do
  if [ -r "$f" ]; then printf '%s: ' "$f"; tr -d '\0' < "$f"; echo; fi
done
uname -a 2>/dev/null || :

printf '\n=== This SSH session ===\n'
id 2>/dev/null || :
printf 'SSH_CONNECTION server port: '
printf '%s\n' "${SSH_CONNECTION:-unknown}" | awk '{print $NF}'

printf '\n=== SSH servers: units, processes, listening TCP ports ===\n'
if command -v systemctl >/dev/null 2>&1; then
  systemctl list-units --all --no-pager --no-legend 2>/dev/null |
    grep -Ei 'ssh|dropbear|developer|devmode|mpfree' || echo 'no matching units'
  for u in mpfree-sshd.service; do
    printf '%s: ' "$u"
    systemctl show "$u" -p LoadState -p ActiveState --value 2>/dev/null | tr '\n' ' '
    echo
  done
fi
for d in /proc/[0-9]*; do
  [ -r "$d/comm" ] || continue
  c="$(cat "$d/comm" 2>/dev/null || :)"
  case "$c" in
    sshd|dropbear) printf 'pid %s %s: ' "$(basename "$d")" "$c"
      tr '\0' ' ' < "$d/cmdline" 2>/dev/null; echo ;;
  esac
done
for f in /proc/net/tcp /proc/net/tcp6; do
  [ -r "$f" ] || continue
  # state 0A = LISTEN; the local port is the hex after the last colon
  awk 'NR>1 && $4=="0A" {n=split($2,a,":"); print a[n]}' "$f" 2>/dev/null | sort -u |
    while read -r h; do printf 'listen %s port %d\n' "$(basename "$f")" "0x$h"; done
done
printf '\n--- sshd_config, active lines only ---\n'
for f in /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf; do
  [ -r "$f" ] || continue
  echo "# $f"
  grep -Ev '^[[:space:]]*(#|$)' "$f" 2>/dev/null || :
done
printf '\n--- /data/mpfree (names only) ---\n'
ls -la /data/mpfree 2>/dev/null || echo 'absent'

printf '\n=== Akai app unit (acvs.service) ===\n'
pid=''
if command -v systemctl >/dev/null 2>&1; then
  systemctl show acvs.service --no-pager \
    -p ActiveState -p ExecStart -p User -p Group -p Restart -p RestartUSec \
    -p NoNewPrivileges -p ProtectSystem -p ProtectHome -p PrivateTmp \
    -p PrivateDevices -p DevicePolicy -p RestrictAddressFamilies \
    -p SystemCallFilter -p CapabilityBoundingSet -p AmbientCapabilities \
    -p ReadWritePaths -p ReadOnlyPaths -p InaccessiblePaths \
    -p LimitMEMLOCK -p LimitRTPRIO -p LimitNOFILE -p LimitSTACK \
    -p CPUAffinity -p MemoryMax -p Delegate 2>&1 || :
  pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)"
  printf 'systemd-run: '; command -v systemd-run || echo 'absent'
  systemctl --version 2>/dev/null | head -n 1
fi

printf '\n=== Akai app process (what an in-process plugin inherits) ===\n'
case "$pid" in
  ''|0|*[!0-9]*) echo 'No active acvs MainPID' ;;
  *)
    # The main PID may be the launcher; report it and its direct children.
    for p in "$pid" $(cat /proc/"$pid"/task/*/children 2>/dev/null); do
      [ -d "/proc/$p" ] || continue
      printf '\n--- pid %s (transient) ---\n' "$p"
      printf 'exe: '; readlink "/proc/$p/exe" 2>/dev/null || echo '?'
      grep -E '^(Name|Uid|Gid|Groups|Threads|VmRSS|VmLck|Cpus_allowed_list|CapEff|CapBnd|NoNewPrivs|Seccomp|Seccomp_filters):' \
        "/proc/$p/status" 2>/dev/null || :
      printf 'cwd: '; readlink "/proc/$p/cwd" 2>/dev/null || echo '?'
      printf 'mount namespace same as pid 1: '
      if [ "$(readlink /proc/1/ns/mnt 2>/dev/null)" = "$(readlink /proc/$p/ns/mnt 2>/dev/null)" ]; then
        echo yes; else echo 'no or unknown'; fi
      if [ -r "/proc/$p/limits" ]; then
        grep -E 'Max (locked memory|realtime priority|stack size|open files)' "/proc/$p/limits" || :
      fi
      printf 'loaded .so files under VST2 Plugins or Synths (names only):\n'
      grep -E 'VST2 Plugins|/Synths/|\.so' "/proc/$p/maps" 2>/dev/null |
        awk '{ $1=$2=$3=$4=$5=""; sub(/^ +/, ""); print }' | grep -Ei 'vst|plugin|synth' | sort -u | head -n 40 || :
      printf '\nthreads: name, last cpu, policy, rt priority\n'
      n=0
      for t in /proc/"$p"/task/[0-9]*; do
        n=$((n+1)); [ "$n" -le 96 ] || { echo 'truncated at 96'; break; }
        name="$(cat "$t/comm" 2>/dev/null || :)"
        awk -v n="$name" '{sub(/^.*\) /, ""); if (NF >= 39) printf "  %-18s cpu=%s policy=%s rt=%s\n", n, $37, $39, $38}' \
          "$t/stat" 2>/dev/null || :
      done
    done
    ;;
esac

printf '\n=== VST2 Plugins folders on mounted drives (names only) ===\n'
awk '{print $2}' /proc/mounts 2>/dev/null | grep -E '^/(media|nvme|sdcard|mnt|run/media)' |
  while read -r m; do
    m="$(printf '%b' "$m")"
    for dir in "$m/VST2 Plugins" "$m/Synths"; do
      [ -d "$dir" ] || continue
      echo "$dir:"
      ls -1 "$dir" 2>/dev/null | head -n 60 | sed 's/^/  /'
    done
  done

printf '\n=== External plugin / enhancement settings (matching lines only) ===\n'
for root in /data /media/az01-internal /root /home; do
  [ -d "$root" ] || continue
  find "$root" -xdev -maxdepth 5 -type f \( -name '*.settings' -o -name '*.prefs' -o -name 'Preferences*' \) 2>/dev/null |
    head -n 20 | while read -r f; do
      echo "# $f"
      grep -Eio '[^<>"]{0,40}(vst|plugin|enhance|developer)[^<>"]{0,40}' "$f" 2>/dev/null | head -n 20 || :
    done
done

printf '\n=== Security modules and integrity (what could refuse a plugin or a helper) ===\n'
for f in /sys/kernel/security/lsm /sys/kernel/security/loadpin/enabled /proc/sys/kernel/modules_disabled; do
  if [ -r "$f" ]; then printf '%s: ' "$f"; cat "$f"; fi
done
command -v systemctl >/dev/null 2>&1 &&
  systemctl list-unit-files --no-pager --no-legend 2>/dev/null | grep -Ei 'signed|verity|integrity' || :
ls -l /usr/bin/az01-signed-fs* /usr/sbin/az01-signed-fs* 2>/dev/null || :
ls /dev/mapper 2>/dev/null || :
printf '\n--- mounts: device, mount point, type, options ---\n'
awk '$2=="/" || $2=="/etc" || $2=="/var" || $2=="/data" || $2=="/tmp" || $2=="/dev/shm" || $2=="/run" || $2 ~ /^\/media/ {print $1, $2, $3, $4}' /proc/mounts 2>/dev/null

printf '\n=== Memory headroom with Akai app running ===\n'
grep -E '^(MemTotal|MemAvailable|Mlocked|SwapTotal|Shmem):' /proc/meminfo 2>/dev/null || :
df -h /dev/shm /tmp /data 2>/dev/null || :

printf '\n=== Recent Akai app log lines about plugins (review before sharing) ===\n'
if command -v journalctl >/dev/null 2>&1; then
  journalctl -u acvs.service -b --no-pager -n 2000 2>/dev/null |
    grep -Ei 'vst|plugin|enhance' | tail -n 40 || :
fi

printf '\nEND MPFREE PLUGIN HOST SURVEY\n'
