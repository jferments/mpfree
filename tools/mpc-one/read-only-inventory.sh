#!/bin/sh
# MPFree MPC One inventory: read-only by design.
# Intended to run on the MPC via: ssh ... 'sh -s' < this-script
# Never install this script on the device or write its output there.
# Some standard read commands can produce routine system access logs.
set -u

section() {
  printf '\n\n========== %s ==========\n' "$1"
  shift
  "$@" 2>&1 || printf '[command unavailable or returned nonzero]\n'
}

printf 'MPFREE MPC ONE READ-ONLY INVENTORY\n'
printf 'No deliberate MPC file writes, mounts, device opens, service changes, or firmware operations.\n'
section 'Identity' sh -c 'id; uname -a; cat /etc/os-release; cat /proc/version'
section 'CPU' sh -c 'cat /proc/cpuinfo; getconf PAGESIZE 2>/dev/null; nproc 2>/dev/null'
section 'RAM and uptime' sh -c 'cat /proc/meminfo; cat /proc/uptime; free -h 2>/dev/null'
section 'Device tree model and compatible' sh -c 'for p in /proc/device-tree/model /proc/device-tree/compatible; do if [ -r "$p" ]; then printf "%s: " "$p"; tr "\000" " " < "$p"; printf "\n"; fi; done'
section 'Filesystems and partitions' sh -c 'cat /proc/partitions; mount; df -hT; cat /proc/mounts'
section 'lsblk' sh -c 'lsblk -o NAME,TYPE,SIZE,FSTYPE,LABEL,MOUNTPOINTS 2>/dev/null || lsblk'
section 'eMMC partition metadata' sh -c 'for p in /sys/class/block/mmcblk0p*/partition /sys/class/block/mmcblk0p*/size /sys/class/block/mmcblk0p*/uevent; do if [ -r "$p" ]; then echo "$p"; cat "$p"; fi; done'
section 'Kernel modules' sh -c 'lsmod 2>/dev/null; ls -ld /lib/modules/* 2>/dev/null'
section 'Processes' sh -c 'ps -eo pid,ppid,comm 2>/dev/null | head -n 160 || ps | head -n 160'
section 'Potential MPC systemd service (may not exist)' sh -c 'systemctl show inmusic-mpc --no-pager -p LoadState -p ActiveState -p SubState -p FragmentPath -p ExecStart -p MainPID 2>/dev/null'
section 'Running services' sh -c 'systemctl list-units --type=service --state=running --no-pager --no-legend 2>/dev/null | head -n 110'
section 'Network (interfaces; not secrets)' sh -c 'ip -br addr; ip route; ss -lnt 2>/dev/null'
section 'ALSA kernel devices' sh -c 'cat /proc/asound/cards 2>/dev/null; cat /proc/asound/pcm 2>/dev/null; cat /proc/asound/modules 2>/dev/null'
section 'ALSA audio enumeration' sh -c 'aplay -l 2>/dev/null; arecord -l 2>/dev/null'
section 'ALSA MIDI enumeration' sh -c 'amidi -l 2>/dev/null; aconnect -l 2>/dev/null | head -n 110'
section 'Audio device nodes' sh -c 'ls -la /dev/snd /dev/snd/* 2>/dev/null | head -n 110'
section 'Graphics/display' sh -c 'cat /proc/fb 2>/dev/null; ls -la /dev/fb* /dev/dri /sys/class/graphics /sys/class/drm /sys/class/backlight 2>/dev/null | head -n 110'
section 'Input device enumeration' sh -c 'cat /proc/bus/input/devices 2>/dev/null; ls -la /dev/input 2>/dev/null; for x in /sys/class/input/event*/device/name; do if [ -r "$x" ]; then printf "%s: " "$x"; cat "$x"; fi; done'
section 'USB buses and devices' sh -c 'lsusb 2>/dev/null; ls -la /sys/bus/usb/devices/ 2>/dev/null | head -n 85'
section 'Relevant app and framework directories' sh -c 'ls -ld /usr/bin/MPC /usr/share/Akai /opt /opt/* /usr/lib/*qt* /usr/lib/*alsa* /usr/share/alsa /usr/share/icons 2>/dev/null | head -n 110'
section 'Relevant executable paths' sh -c 'for c in MPC jackd pipewire pulseaudio aplay amidi aconnect arecord speaker-test ffmpeg gst-launch-1.0 weston Xorg python3 readelf file; do command -v "$c" 2>/dev/null || :; done'
section 'Package tool availability' sh -c 'for c in opkg rpm dpkg; do if command -v "$c" >/dev/null 2>&1; then echo "$c present"; fi; done'
section 'Realtime tuning and interrupts' sh -c 'cat /proc/sys/kernel/sched_rt_runtime_us /proc/sys/kernel/sched_rt_period_us 2>/dev/null; head -n 75 /proc/interrupts 2>/dev/null'
section 'Potential MPC application assets (names/paths only)' sh -c 'for d in /usr/bin /usr/share/Akai /opt; do if [ -d "$d" ]; then find "$d" -maxdepth 3 \( -iname "*mpc*" -o -iname "*akai*" -o -iname "*inmusic*" \) -print 2>/dev/null | head -n 100; fi; done'
printf '\n\nEND OF READ-ONLY INVENTORY\n'
