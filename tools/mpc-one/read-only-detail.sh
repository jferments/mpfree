#!/bin/sh
# MPFree MPC One: focused second-pass READ-ONLY inspection.
# Streamed over SSH stdin. No persistent writes or app/resource takeover.
set -u
section() { printf '\n\n========== %s ==========\n' "$1"; shift; "$@" 2>&1 || echo '[some subcommands failed or unavailable]'; }
safe_file() { if [ -r "$1" ]; then printf '%s: ' "$1"; cat "$1" 2>/dev/null || :; printf '\n'; fi; }

process_names() {
  # Portable /proc instead of GNU ps -eo (not necessarily supported by BusyBox).
  for d in /proc/[0-9]*; do
    [ -r "$d/comm" ] || continue
    printf '%s ' "$(basename "$d")"
    cat "$d/comm" 2>/dev/null || :
  done | head -n 180
}
service_metadata() {
  command -v systemctl >/dev/null 2>&1 || return 0
  for unit in acvs.service az01-network-midi.service az01-script-runner.service; do
    echo "--- $unit ---"
    systemctl show "$unit" --no-pager \
      -p LoadState -p ActiveState -p SubState -p Type -p FragmentPath \
      -p MainPID -p ExecStart -p ExecMainPID 2>/dev/null || :
  done
}
app_hardware_links() {
  command -v systemctl >/dev/null 2>&1 || return 0
  pid="$(systemctl show acvs.service -p MainPID 2>/dev/null | sed -n 's/^MainPID=//p' | head -n 1)"
  case "$pid" in ''|0|*[!0-9]*) echo 'acvs MainPID unavailable'; return 0;; esac
  [ -d "/proc/$pid/fd" ] || return 0
  echo "acvs MainPID=$pid (hardware device links only)"
  count=0
  for x in /proc/"$pid"/fd/*; do
    [ -L "$x" ] || continue
    target="$(readlink "$x" 2>/dev/null || :)"
    case "$target" in
      /dev/snd/*|/dev/input/*|/dev/dri/*|/dev/fb*|/dev/hidraw*|/dev/ttyS*)
        printf 'fd/%s -> %s\n' "$(basename "$x")" "$target"
        count=$((count+1))
        [ "$count" -lt 100 ] || break
        ;;
    esac
  done
}
alsa_metadata() {
  for x in /proc/asound/cards /proc/asound/pcm \
           /proc/asound/card0/id /proc/asound/card1/id \
           /proc/asound/card0/stream0 \
           /proc/asound/card1/pcm0p/info /proc/asound/card1/pcm0c/info; do
    safe_file "$x"
  done
  for x in /sys/class/sound/controlC0/device/driver \
           /sys/class/sound/controlC1/device/driver; do
    [ -L "$x" ] && ls -ld "$x" || :
  done
}
display_metadata() {
  for x in /sys/class/drm/card0-DSI-1/status \
           /sys/class/drm/card0-DSI-1/enabled \
           /sys/class/drm/card0-DSI-1/modes \
           /sys/class/graphics/fb0/virtual_size \
           /sys/class/graphics/fb0/bits_per_pixel \
           /sys/class/backlight/mipi-backlight/max_brightness \
           /sys/class/backlight/mipi-backlight/brightness \
           /sys/class/drm/card1/device/uevent; do
    safe_file "$x"
  done
}
input_metadata() {
  for d in /sys/class/input/event*/device; do
    [ -d "$d" ] || continue
    for x in "$d/name" "$d/id/bustype" "$d/id/vendor" "$d/id/product" \
             "$d/capabilities/ev" "$d/capabilities/key" "$d/capabilities/abs"; do
      safe_file "$x"
    done
  done
}
usb_metadata() {
  # Never read sysfs USB serial or raw binary descriptors.
  for d in /sys/bus/usb/devices/*; do
    [ -r "$d/idVendor" ] || continue
    echo "---- $(basename "$d") ----"
    for x in "$d/idVendor" "$d/idProduct" "$d/manufacturer" "$d/product" \
             "$d/bcdDevice" "$d/speed" "$d/bDeviceClass" "$d/bNumInterfaces"; do
      safe_file "$x"
    done
  done
}
cpu_metadata() {
  for d in /sys/devices/system/cpu/cpu[0-9]*/cpufreq; do
    [ -d "$d" ] || continue
    for x in "$d/scaling_governor" "$d/scaling_cur_freq" "$d/cpuinfo_max_freq"; do
      safe_file "$x"
    done
  done
}
elf_metadata() {
  if [ -r /usr/bin/MPC ] && command -v od >/dev/null 2>&1; then
    echo '/usr/bin/MPC — first 32 ELF header bytes only:'
    head -c 32 /usr/bin/MPC | od -An -tx1
  fi
  ls -ld /lib/ld-linux* /lib/libc* /usr/lib/libasound* 2>/dev/null | head -n 30 || :
}
network_metadata() { ip addr 2>/dev/null || :; ip route 2>/dev/null || :; }

printf 'MPFREE FOCUSED READ-ONLY HARDWARE INVENTORY\n'
section 'Process names' process_names
section 'Factory application service metadata' service_metadata
section 'Factory process hardware fd symlinks (no opens)' app_hardware_links
section 'ALSA device metadata' alsa_metadata
section 'Display and DRM properties' display_metadata
section 'Input event capabilities' input_metadata
section 'USB vendor/product identifiers (no serials)' usb_metadata
section 'CPU frequency' cpu_metadata
section 'ELF header and loader path names' elf_metadata
section 'BusyBox-compatible network metadata' network_metadata
printf '\nEND OF FOCUSED INVENTORY\n'
