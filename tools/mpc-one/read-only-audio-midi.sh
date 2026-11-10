#!/bin/sh
# MPFree MPC One read-only audio + MIDI implementation survey (POSIX sh, BusyBox ash).
#
# Purpose: collect what the direct-ALSA engine (ADR-0002) and the internal
# controller MIDI backend need, while the factory app keeps running untouched.
#
# SAFETY CONTRACT (checked in CI):
# - Reads only /proc, /sys and /proc/device-tree text/attribute files, plus
#   small config files under /etc and /usr/share/alsa.
# - Never opens /dev/snd/*, never runs aplay/arecord/amixer/amidi/aconnect,
#   never sends MIDI, never writes to sysfs/procfs/debugfs.
# - Never mounts anything (debugfs is read only if ALREADY mounted), never
#   loads modules, never changes services, scheduling, affinity or cpufreq.
# - Writes nothing on the MPC. Output goes to stdout, which the host saves.
# - Uses only BusyBox applets known to work on az0x: no `od -A/-t`, no
#   `head -c`, no `tr` character classes.
set -u

section() { printf '\n\n========== %s ==========\n' "$1"; shift; "$@" 2>&1 || echo '[some optional reads unavailable]'; }
read_file() {
  if [ -r "$1" ]; then
    printf '%s\n' "--- $1"
    head -n "${2:-80}" "$1" 2>/dev/null || :
  else
    printf 'unavailable (not necessarily unsupported): %s\n' "$1"
  fi
}
have() { command -v "$1" >/dev/null 2>&1; }

alsa_procfs() {
  for f in /proc/asound/version /proc/asound/cards /proc/asound/devices \
           /proc/asound/pcm /proc/asound/timers /proc/asound/modules; do
    read_file "$f" 60
  done
  for card in /proc/asound/card[0-9]*; do
    [ -d "$card" ] || continue
    printf '\n### %s contents: ' "$card"
    ls "$card" 2>/dev/null | tr '\n' ' '
    printf '\n'
    read_file "$card/id" 3
  done
  # ACVA PCM capability/info and current factory stream (same reads as --runtime).
  for dir in p c; do
    read_file "/proc/asound/card1/pcm0$dir/info" 30
    for name in hw_params sw_params status; do
      read_file "/proc/asound/card1/pcm0$dir/sub0/$name" 30
    done
    # Present only on some kernels/configs; harmless text files.
    read_file "/proc/asound/card1/pcm0$dir/xrun_debug" 5
    read_file "/proc/asound/card1/pcm0$dir/sub0/prealloc" 5
    read_file "/proc/asound/card1/pcm0$dir/sub0/prealloc_max" 5
  done
}

sysfs_sound() {
  for d in /sys/class/sound/*; do
    [ -e "$d" ] || continue
    printf '%s dev=' "$(basename "$d")"
    cat "$d/dev" 2>/dev/null | tr '\n' ' ' || :
    printf 'device=' ; readlink "$d/device" 2>/dev/null | tr '\n' ' ' || :
    printf 'driver=' ; readlink "$d/device/driver" 2>/dev/null || echo '-'
  done
  for d in /sys/class/sound/card*; do
    [ -d "$d" ] || continue
    for a in id number; do
      [ -r "$d/$a" ] && printf '%s/%s=%s\n' "$d" "$a" "$(cat "$d/$a" 2>/dev/null)"
    done
  done
  return 0
}

dt_print_node() {
  node="$1"
  printf '\n### DT node %s\n' "${node#/proc/device-tree}"
  for p in "$node"/*; do
    [ -f "$p" ] || continue
    name="$(basename "$p")"
    case "$name" in
      compatible|status|name|label|*-names|*format*|sound-name-prefix|model|simple-audio-card,name)
        printf '  %s = ' "$name"; tr '\000' ' ' < "$p" 2>/dev/null; printf '\n' ;;
      phandle|linux,phandle) : ;;
      # Never print device identifiers (the root node has serial-number).
      *serial*|*mac-address*|*uuid*) printf '  %s = [redacted]\n' "$name" ;;
      *)
        # Numeric/binary property: print raw bytes as octal (BusyBox-safe), capped.
        printf '  %s (octal bytes) = ' "$name"
        od -b "$p" 2>/dev/null | head -n 4 | tr '\n' ' '
        printf '\n' ;;
    esac
  done
}

device_tree_audio() {
  [ -d /proc/device-tree ] || { echo 'no /proc/device-tree'; return 0; }
  read_file /proc/device-tree/model 2
  printf '\n'
  have find || { echo 'find unavailable; skipping device-tree walk'; return 0; }
  n=0
  # Only nodes whose compatible names audio/I2S/codec/DMA hardware.
  # /proc/device-tree is a symlink: the trailing slash makes find descend into it.
  find /proc/device-tree/ -maxdepth 4 -name compatible 2>/dev/null | while read -r c; do
    if tr '\000' ' ' < "$c" 2>/dev/null |
       grep -i -q -E 'acva|jp07|inmusic|i2s|audio|codec|sound|pdma|dmac|spdif'; then
      n=$((n+1))
      [ "$n" -le 40 ] || { echo 'DT audio node list truncated at 40'; break; }
      dt_print_node "$(dirname "$c")"
    fi
  done
}

kernel_modules() {
  if [ -r /proc/modules ]; then
    echo 'Loaded modules matching sound/USB/MIDI/DMA:'
    grep -i -E 'snd|soc|i2s|midi|usb|dma|acva|jp07|inmusic' /proc/modules | head -n 60
  fi
  for m in /sys/module/snd* /sys/module/usbcore /sys/module/usbmon; do
    [ -d "$m" ] || continue
    [ -d "$m/parameters" ] || { printf '%s (no parameters)\n' "$(basename "$m")"; continue; }
    for p in "$m"/parameters/*; do
      [ -r "$p" ] || continue
      printf '%s.%s=%s\n' "$(basename "$m")" "$(basename "$p")" "$(head -n 1 "$p" 2>/dev/null)"
    done
  done
  return 0
}

kernel_config() {
  if [ -r /proc/config.gz ] && have gzip; then
    gzip -dc /proc/config.gz 2>/dev/null |
      grep -E '^(CONFIG_(HZ|HZ_[0-9]+|PREEMPT[A-Z_]*|HIGH_RES_TIMERS|NO_HZ[A-Z_]*|RT_GROUP_SCHED|CGROUP_SCHED|CPU_ISOLATION|CPU_FREQ_DEFAULT_GOV_[A-Z]+|CPU_IDLE|SND_[A-Z0-9_]*|USB_MON|DEBUG_FS|DEBUG_FS_ALLOW_[A-Z]+|SCHED_DEBUG|FTRACE|KEXEC))=|^# CONFIG_(RT_GROUP_SCHED|USB_MON|DEBUG_FS|HIGH_RES_TIMERS|NO_HZ_FULL|SND_PCM_XRUN_DEBUG) is not set' |
      head -n 200
  else
    echo '/proc/config.gz unavailable (CONFIG_IKCONFIG_PROC not set or gzip missing)'
  fi
}

debugfs_if_mounted() {
  if grep -q ' debugfs ' /proc/mounts 2>/dev/null; then
    dfs="$(awk '$3=="debugfs"{print $2; exit}' /proc/mounts)"
    printf 'debugfs already mounted at %s\n' "$dfs"
    for f in components dais platforms; do
      read_file "$dfs/asoc/$f" 60
    done
    if [ -d "$dfs/usb/usbmon" ]; then
      echo 'usbmon debugfs directory present:'
      ls "$dfs/usb/usbmon" 2>/dev/null | tr '\n' ' '; printf '\n'
    else
      echo 'usbmon debugfs directory absent'
    fi
  else
    echo 'debugfs is NOT mounted. This survey does not mount it.'
  fi
}

usbmon_availability() {
  if [ -d /sys/module/usbmon ]; then echo 'usbmon: loaded or built in (/sys/module/usbmon exists)'; else echo 'usbmon: not loaded or not built in'; fi
  rel="$(uname -r 2>/dev/null)"
  if [ -n "$rel" ] && [ -d "/lib/modules/$rel" ]; then
    if have find; then
      find "/lib/modules/$rel" -name 'usbmon*' 2>/dev/null | head -n 5
    fi
    [ -r "/lib/modules/$rel/modules.builtin" ] &&
      grep -E 'usbmon|usb/mon' "/lib/modules/$rel/modules.builtin" 2>/dev/null | head -n 5
  fi
  return 0
}

irq_layout() {
  read_file /proc/interrupts 120
  echo
  echo 'Affinity of audio/DMA/USB-related IRQ lines:'
  awk -F: 'NR>1 && /i2s|I2S|dma|DMA|usb|USB|dwc|ehci|ohci|acva|sound/ {gsub(/ /,"",$1); print $1}' \
    /proc/interrupts 2>/dev/null | while read -r irq; do
    case "$irq" in ''|*[!0-9]*) continue ;; esac
    printf 'irq %s smp_affinity_list=%s effective=%s\n' "$irq" \
      "$(cat "/proc/irq/$irq/smp_affinity_list" 2>/dev/null)" \
      "$(cat "/proc/irq/$irq/effective_affinity_list" 2>/dev/null)"
  done
}

kernel_threads_rt() {
  echo 'Threaded IRQ and timer kthreads (PREEMPT_RT): policy 1=FIFO 2=RR 0=OTHER'
  n=0
  for d in /proc/[0-9]*; do
    [ -r "$d/comm" ] || continue
    comm="$(cat "$d/comm" 2>/dev/null)"
    case "$comm" in
      irq/*|ktimers*|ksoftirqd*|rcuc*|rcub*|migration*|irq_work*) ;;
      *) continue ;;
    esac
    n=$((n+1))
    [ "$n" -le 200 ] || { echo 'truncated at 200'; break; }
    printf '%s pid=%s ' "$comm" "$(basename "$d")"
    awk '{sub(/^.*\) /, ""); if (NF >= 39) printf "last_cpu=%s rt_priority=%s policy=%s ", $37, $38, $39}' \
      "$d/stat" 2>/dev/null || :
    grep '^Cpus_allowed_list:' "$d/status" 2>/dev/null | tr '\t' ' ' || echo
  done
}

cpu_power() {
  for f in /sys/devices/system/cpu/online /sys/devices/system/cpu/isolated \
           /sys/devices/system/cpu/nohz_full; do
    [ -r "$f" ] && printf '%s: %s\n' "$f" "$(cat "$f")"
  done
  for c in /sys/devices/system/cpu/cpu[0-9]*; do
    [ -d "$c/cpufreq" ] || continue
    printf '%s: ' "$(basename "$c")"
    for a in scaling_driver scaling_governor scaling_cur_freq scaling_min_freq scaling_max_freq cpuinfo_transition_latency; do
      printf '%s=%s ' "$a" "$(cat "$c/cpufreq/$a" 2>/dev/null)"
    done
    printf '\n'
  done
  for c in /sys/devices/system/cpu/cpu0/cpufreq /sys/devices/system/cpu/cpufreq/policy0; do
    for a in scaling_available_frequencies scaling_available_governors related_cpus; do
      [ -r "$c/$a" ] && printf '%s/%s: %s\n' "$c" "$a" "$(cat "$c/$a")"
    done
  done
  read_file /sys/devices/system/cpu/cpuidle/current_driver 2
  for s in /sys/devices/system/cpu/cpu0/cpuidle/state[0-9]*; do
    [ -d "$s" ] || continue
    printf '%s name=%s latency_us=%s disable=%s\n' "$(basename "$s")" \
      "$(cat "$s/name" 2>/dev/null)" "$(cat "$s/latency" 2>/dev/null)" "$(cat "$s/disable" 2>/dev/null)"
  done
  return 0
}

rt_limits() {
  for f in /proc/sys/kernel/sched_rt_runtime_us /proc/sys/kernel/sched_rt_period_us \
           /proc/sys/kernel/sched_rr_timeslice_ms /proc/sys/kernel/timer_migration \
           /proc/sys/kernel/sched_util_clamp_min_rt_default /proc/sys/vm/swappiness \
           /proc/sys/vm/overcommit_memory /proc/sys/kernel/perf_event_paranoid; do
    [ -r "$f" ] && printf '%s = %s\n' "$f" "$(cat "$f")"
  done
  echo
  echo 'cgroup layout (RT_GROUP_SCHED can block SCHED_FIFO in non-root cgroups):'
  grep cgroup /proc/mounts 2>/dev/null | head -n 20 || :
  for f in /sys/fs/cgroup/cpu.rt_runtime_us /sys/fs/cgroup/cpu/cpu.rt_runtime_us \
           /sys/fs/cgroup/cgroup.controllers /sys/fs/cgroup/cpuset.cpus.effective; do
    [ -r "$f" ] && printf '%s = %s\n' "$f" "$(cat "$f")"
  done
  pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)"
  case "$pid" in ''|0|*[!0-9]*) echo 'acvs MainPID unknown';;
    *) printf 'acvs cgroup: '; cat "/proc/$pid/cgroup" 2>/dev/null | head -n 5
       grep -E 'Max (locked memory|realtime priority|realtime timeout)' "/proc/$pid/limits" 2>/dev/null ;;
  esac
  echo
  echo 'PAM / systemd RT and memlock limits (config text only):'
  for f in /etc/security/limits.conf /etc/security/limits.d/*; do
    [ -r "$f" ] || continue
    grep -v '^[[:space:]]*#' "$f" 2>/dev/null | grep -i -E 'rtprio|memlock|nice' | sed "s|^|$f: |"
  done
  have systemctl && systemctl show --no-pager -p DefaultLimitRTPRIO -p DefaultLimitMEMLOCK -p DefaultLimitRTTIME 2>/dev/null
  return 0
}

alsa_userspace() {
  echo 'alsa-lib and tools present (paths only, nothing executed):'
  ls -l /usr/lib/libasound.so* /lib/libasound.so* 2>/dev/null
  for c in aplay arecord amixer alsactl alsaloop speaker-test amidi aconnect aseqdump aseqnet evtest jackd pipewire; do
    p="$(command -v "$c" 2>/dev/null)" && printf '  %s -> %s\n' "$c" "$p"
  done
  read_file /etc/asound.conf 80
  [ -d /usr/share/alsa ] && { printf '/usr/share/alsa: '; ls /usr/share/alsa 2>/dev/null | tr '\n' ' '; printf '\n'; }
  for f in /usr/share/alsa/cards/ACVA.conf /var/lib/alsa/asound.state /etc/asound.state; do
    [ -r "$f" ] && read_file "$f" 120
  done
  [ -d /usr/share/alsa/ucm2 ] && { echo 'UCM2 profiles present:'; ls /usr/share/alsa/ucm2 2>/dev/null | head -n 30; }
  return 0
}

midi_procfs() {
  for f in /proc/asound/card0/midi0 /proc/asound/card0/usbmidi /proc/asound/card0/usbid \
           /proc/asound/card0/usbbus /proc/asound/seq/clients /proc/asound/seq/queues \
           /proc/asound/seq/timer; do
    read_file "$f" 120
  done
}

usb_midi_device() {
  for dev in /sys/bus/usb/devices/[0-9]*-[0-9]*; do
    [ -r "$dev/idVendor" ] || continue
    [ "$(cat "$dev/idVendor" 2>/dev/null)" = "09e8" ] || continue
    printf '### USB device %s\n' "$(basename "$dev")"
    for a in idVendor idProduct bcdDevice bDeviceClass version speed manufacturer product bMaxPower bNumConfigurations bConfigurationValue; do
      [ -r "$dev/$a" ] && printf '  %s=%s\n' "$a" "$(head -n 1 "$dev/$a" 2>/dev/null)"
    done
    # Cached raw descriptors (no USB traffic). Contains MIDI jack/endpoint layout.
    if [ -r "$dev/descriptors" ]; then
      echo '  raw descriptors (octal bytes, first 64 lines):'
      od -b "$dev/descriptors" 2>/dev/null | head -n 64
    fi
    for intf in "$dev"/*:*; do
      [ -d "$intf" ] || continue
      printf '  interface %s class=%s subclass=%s eps=%s driver=%s\n' "$(basename "$intf")" \
        "$(cat "$intf/bInterfaceClass" 2>/dev/null)" "$(cat "$intf/bInterfaceSubClass" 2>/dev/null)" \
        "$(cat "$intf/bNumEndpoints" 2>/dev/null)" "$(basename "$(readlink "$intf/driver" 2>/dev/null)" 2>/dev/null)"
      for ep in "$intf"/ep_*; do
        [ -d "$ep" ] || continue
        printf '    %s type=%s dir=%s maxpkt=%s interval=%s\n' "$(basename "$ep")" \
          "$(cat "$ep/type" 2>/dev/null)" "$(cat "$ep/direction" 2>/dev/null)" \
          "$(cat "$ep/wMaxPacketSize" 2>/dev/null)" "$(cat "$ep/interval" 2>/dev/null)"
      done
    done
  done
  return 0
}

input_devices() {
  # Hardware keys/encoders that might be evdev rather than MIDI.
  read_file /proc/bus/input/devices 120
}

printf 'MPFREE MPC ONE READ-ONLY AUDIO+MIDI SURVEY V1\n'
printf 'kernel: %s\n' "$(uname -r 2>/dev/null)"
section 'ALSA procfs: cards, PCM info, current ACVA stream' alsa_procfs
section 'sysfs sound class' sysfs_sound
section 'Device tree audio/I2S/DMA nodes' device_tree_audio
section 'Sound/USB kernel modules and parameters' kernel_modules
section 'Kernel config (audio/RT relevant symbols)' kernel_config
section 'debugfs (only if already mounted)' debugfs_if_mounted
section 'usbmon availability (nothing loaded)' usbmon_availability
section 'IRQ table and audio/USB IRQ affinity' irq_layout
section 'RT kernel threads' kernel_threads_rt
section 'CPU frequency and idle states' cpu_power
section 'RT scheduling limits and cgroups' rt_limits
section 'ALSA userspace (files only)' alsa_userspace
section 'MIDI procfs (rawmidi counters, sequencer)' midi_procfs
section 'Internal USB MIDI device descriptors' usb_midi_device
section 'Input devices' input_devices
printf '\nEND OF AUDIO+MIDI SURVEY\n'
