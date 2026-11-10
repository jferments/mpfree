#!/bin/sh
# MPFree boot menu survey - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --boot-menu-survey.
#
# Answers what a boot menu (rust/mpfree-splash --menu) needs to know about
# the running stock firmware before anything is installed:
#   - can /dev/fb0 be drawn on, and who holds the display and MIDI devices;
#   - which ALSA card/device is the internal controller, and its subdevices;
#   - how Akai's app (acvs.service) and the USB script runner start, restart
#     and depend on other units, and when they started in this boot;
#   - whether kexec is available and /run has room;
#   - how USB sticks are mounted (noexec or not).
#
# It reads procfs, sysfs, unit properties and unit text, resolves /proc/*/fd
# symlinks (readlink only), and lists names. It NEVER opens a display, sound,
# input or block device, never mounts, writes, starts, stops or restarts
# anything, and never reads file contents from a USB stick.
set -u
printf 'MPFREE BOOT MENU SURVEY - READ ONLY\n'
section() { printf '\n=== %s ===\n' "$1"; }
show() { for f in "$@"; do [ -r "$f" ] && printf '%s: %s\n' "$f" "$(cat "$f" 2>/dev/null | tr '\n' ' ')"; done; }

section 'Release'
cat /proc/version 2>/dev/null || :
grep -E '^(NAME|VERSION|VERSION_ID|PRETTY_NAME)=' /etc/os-release 2>/dev/null || :
printf 'uptime: %s\n' "$(cat /proc/uptime 2>/dev/null)"

section 'Framebuffer (sysfs only)'
ls -l /dev/fb* 2>&1 || :
for d in /sys/class/graphics/fb*; do
  [ -d "$d" ] || continue
  show "$d/name" "$d/modes" "$d/mode" "$d/virtual_size" "$d/stride" \
       "$d/bits_per_pixel" "$d/pan" "$d/rotate" "$d/state"
done
grep -i -E 'fb|drm' /proc/fb 2>/dev/null || cat /proc/fb 2>/dev/null || :
show /sys/module/drm_kms_helper/parameters/fbdev_emulation /sys/module/drm/parameters/fbdev_emulation
for c in /sys/class/drm/card*-*; do
  [ -d "$c" ] || continue
  show "$c/status" "$c/enabled" "$c/dpms"
done

section 'ALSA cards and the controller'
cat /proc/asound/cards 2>/dev/null || :
ls -l /dev/snd 2>&1 || :
for m in /proc/asound/card*/midi*; do
  [ -r "$m" ] || continue
  printf '%s\n' "--- $m"
  cat "$m" 2>/dev/null || :
done
for id in /proc/asound/card*/id; do show "$id"; done

section 'Who holds display, MIDI and input devices (readlink of /proc/*/fd)'
for p in /proc/[0-9]*; do
  [ -d "$p/fd" ] || continue
  hits=''
  for fd in "$p"/fd/*; do
    l=$(readlink "$fd" 2>/dev/null) || continue
    case "$l" in
      /dev/fb*|/dev/dri/*|/dev/snd/midi*|/dev/snd/seq|/dev/input/*|/dev/tty0|/dev/tty[0-9]*) hits="$hits $l" ;;
    esac
  done
  [ -n "$hits" ] && printf 'pid=%s comm=%s unit=%s:%s\n' "${p#/proc/}" "$(cat "$p/comm" 2>/dev/null)" \
    "$(sed -n 's/^0::\(.*\)$/\1/p' "$p/cgroup" 2>/dev/null | head -n 1)" "$hits"
done

section 'Services: Akai app, script runner, network MIDI, persistent SSH'
if command -v systemctl >/dev/null 2>&1; then
  for u in acvs.service az01-script-runner.service az01-network-midi.service mpfree-sshd.service; do
    printf '%s\n' "--- $u"
    systemctl show "$u" --no-pager -p Id -p LoadState -p ActiveState -p SubState -p UnitFileState \
      -p FragmentPath -p ExecStart -p ExecStop -p Restart -p RestartUSec -p KillMode -p TimeoutStopUSec \
      -p Requires -p Wants -p BindsTo -p PartOf -p Requisite -p Conflicts -p After -p Before -p WantedBy -p RequiredBy \
      -p ExecMainStartTimestampMonotonic -p ActiveEnterTimestampMonotonic -p NRestarts 2>&1 || :
  done
  printf '%s\n' '--- unit text: acvs.service'
  systemctl cat acvs.service --no-pager 2>&1 || :
  printf '%s\n' '--- unit text: az01-script-runner.service'
  systemctl cat az01-script-runner.service --no-pager 2>&1 || :
  printf '%s\n' '--- default target and boot timing'
  systemctl get-default 2>&1 || :
  systemctl show --no-pager -p UserspaceTimestampMonotonic -p FinishTimestampMonotonic 2>&1 || :
  if command -v systemd-analyze >/dev/null 2>&1; then
    systemd-analyze 2>&1 || :
    systemd-analyze critical-chain acvs.service --no-pager 2>&1 | head -n 30 || :
  else
    printf 'systemd-analyze: not installed\n'
  fi
else
  printf 'systemctl=unavailable\n'
fi

section 'USB script runner and autorun (names, sizes and match counts only)'
for f in /usr/bin/az01-script-runner /usr/bin/az01-launch-MPC; do
  [ -e "$f" ] || { printf '%s: absent\n' "$f"; continue; }
  ls -l "$f" 2>&1
  printf '%s: lines mentioning autorun: %s, mentioning /media: %s\n' "$f" \
    "$(grep -c -i autorun "$f" 2>/dev/null)" "$(grep -c /media "$f" 2>/dev/null)"
done
if command -v journalctl >/dev/null 2>&1; then
  printf '%s\n' '--- script runner log this boot (first 40 lines; may name USB sticks)'
  journalctl -b -u az01-script-runner.service --no-pager -o short-monotonic 2>&1 | head -n 40 || :
fi

section 'Removable media mounts (options only)'
grep -E ' /media| /run/media| /mnt' /proc/mounts 2>/dev/null || printf 'no stick mounted\n'

section 'kexec and /run'
ls -l /usr/sbin/kexec 2>&1 || :
show /proc/sys/kernel/kexec_load_disabled /sys/kernel/kexec_loaded /sys/kernel/security/lockdown
df -k /run 2>/dev/null | tail -n 1 || :

printf '\nEND OF BOOT MENU SURVEY (nothing was changed)\n'
