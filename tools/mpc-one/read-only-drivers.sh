#!/bin/sh
# MPFree driver survey - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --drivers (Akai's system, port 22)
# and --stick-drivers (MPFree's system on the stick, port 2222), so the kernels
# can be compared line by line. It is step 1 of MPFree's own kernel
# (docs/dev/hardware/own-kernel.md): what does each kernel rely on?
#
# Answers: which driver is bound to every device on every bus, which device-tree
# node each device comes from, which modules are loaded, what the sound cards,
# screen, backlight, inputs and regulators are, what the kernel said about
# firmware and failed or deferred probes, and how the kernel was built
# (/proc/config.gz, including CONFIG_MODULE_SIG_FORCE).
#
# It reads procfs and sysfs, debugfs only if it is already mounted, and the
# kernel log. It never mounts, never loads or unloads a module, never binds or
# unbinds a driver, never changes a setting and never writes a file. It prints
# no MAC address, serial number or partition UUID.
set -u
R="${MPFREE_FAKE_ROOT:-}"
printf 'MPFREE DRIVER SURVEY - READ ONLY\n'
printf 'MAC addresses, serial numbers and UUIDs are replaced with <MAC>, <serial> and <uuid>.\n'

# The build settings that matter for MPFree's own kernel. offline-own-kernel-survey.py
# selects the same symbols from Akai's kernel file; a test keeps the two lists equal.
SELECT='^(# )?CONFIG_(LOCALVERSION|CC_VERSION_TEXT|PREEMPT[A-Z_]*|HZ[A-Z_0-9]*|NO_HZ[A-Z_]*|HIGH_RES_TIMERS|SMP|NR_CPUS|HIGHMEM|VMSPLIT_[0-9A-Z_]+|ARM_LPAE|VFP[A-Z0-9_]*|NEON|CPU_FREQ[A-Z_]*|ARM_RK3288_CPUFREQ|CPU_IDLE[A-Z_]*|MODULES|MODULE_SIG[A-Z_]*|MODULE_COMPRESS[A-Z_]*|KEXEC[A-Z_]*|SECURITY_LOADPIN[A-Z_]*|SECURITY_LOCKDOWN_LSM|LSM|IKCONFIG[A-Z_]*|CMDLINE[A-Z_]*|BLK_DEV_INITRD|MMC[A-Z0-9_]*|SND_SOC|SND_SOC_ROCKCHIP[A-Z0-9_]*|SND_SOC_SIMPLE[A-Z_]*|SND_SIMPLE[A-Z_]*|SND_SOC_GPIO[A-Z_]*|SND_SOC_SPDIF|SND_USB_AUDIO|SND_HRTIMER|SND_SEQ[A-Z_]*|SND_RAWMIDI|DRM|DRM_ROCKCHIP|ROCKCHIP_[A-Z0-9_]+|DRM_DW_MIPI_DSI|DRM_PANEL[A-Z0-9_]*|DRM_PANFROST|DRM_LIMA|DRM_FBDEV_EMULATION|FB|FRAMEBUFFER_CONSOLE|BACKLIGHT_[A-Z0-9_]+|TOUCHSCREEN_ILI210X|INPUT_TOUCHSCREEN|USB_DWC2[A-Z_]*|USB_EHCI[A-Z_]*|USB_OHCI[A-Z_]*|USB_STORAGE|USB_UAS|STMMAC_ETH|STMMAC_PLATFORM|DWMAC_ROCKCHIP|REALTEK_PHY|CFG80211|PSTORE[A-Z_]*|ANDROID_RAM_CONSOLE|MAGIC_SYSRQ|PRINTK_TIME|IKHEADERS|DEBUG_FS)(=| is not set)'

section() { printf '\n=== %s ===\n' "$1"; }
redact() {
  sed -e 's/[0-9A-Fa-f][0-9A-Fa-f]\(:[0-9A-Fa-f][0-9A-Fa-f]\)\{5\}/<MAC>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{4\}-[0-9A-Fa-f]\{12\}/<uuid>/g' \
      -e 's/[0-9A-Fa-f]\{8\}-[0-9A-Fa-f]\{2\}/<uuid>/g' \
      -e 's/\([A-Za-z_.]*[Ss][Ee][Rr][Ii][Aa][Ll][A-Za-z_.]*[=:] *\)[^ ,]*/\1<serial>/g'
}
value() { [ -r "$1" ] && head -n 1 "$1" 2>/dev/null; return 0; }
link_name() { [ -L "$1" ] && basename "$(readlink "$1")"; return 0; }
# The device-tree path of a device's node, from its of_node link (.../devicetree/base/PATH).
node_of() {
  [ -L "$1/of_node" ] || return 0
  n=$(readlink "$1/of_node")
  case "$n" in
    */devicetree/base) printf '/' ;;
    */devicetree/base/*) printf '/%s' "${n#*/devicetree/base/}" ;;
  esac
  return 0
}
# A file of NUL-separated strings (a device-tree property), on one line.
strings_in() { [ -r "$1" ] && tr '\000' ' ' < "$1" | sed 's/ *$//'; return 0; }
compat_of() { strings_in "$1/of_node/compatible"; }
kernel_log() { if [ -n "$R" ]; then cat "$R/kernel-log" 2>/dev/null; else dmesg 2>/dev/null; fi; }

section 'Kernel'
printf 'release: %s\n' "$(value "$R/proc/sys/kernel/osrelease")"
printf 'version: %s\n' "$(value "$R/proc/sys/kernel/version")"
printf 'uptime: %s s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"
printf 'command line: %s\n' "$(value "$R/proc/cmdline" | redact)"
printf 'model: %s\n' "$(strings_in "$R/sys/firmware/devicetree/base/model")"
printf 'compatible: %s\n' "$(strings_in "$R/sys/firmware/devicetree/base/compatible")"
if [ -r "$R/proc/sys/kernel/loadpin/enforce" ]; then
  printf 'loadpin enforce: %s\n' "$(value "$R/proc/sys/kernel/loadpin/enforce")"
else
  printf 'loadpin enforce: (no /proc/sys/kernel/loadpin/enforce)\n'
fi
printf 'lockdown: %s\n' "$(value "$R/sys/kernel/security/lockdown")"
printf 'kexec_load_disabled: %s\n' "$(value "$R/proc/sys/kernel/kexec_load_disabled")"

section 'Build settings that matter for our own kernel (/proc/config.gz)'
have_config=no
if [ -r "$R/proc/config.gz" ] && gzip -dc "$R/proc/config.gz" >/dev/null 2>&1; then
  have_config=yes
  printf 'lines: %s, built in (=y): %s, modules (=m): %s\n' \
    "$(gzip -dc "$R/proc/config.gz" | grep -c '')" \
    "$(gzip -dc "$R/proc/config.gz" | grep -c '^CONFIG_[A-Za-z0-9_]*=y$')" \
    "$(gzip -dc "$R/proc/config.gz" | grep -c '^CONFIG_[A-Za-z0-9_]*=m$')"
  if command -v sha256sum >/dev/null 2>&1; then
    printf 'sha256 of the whole config: %s\n' "$(gzip -dc "$R/proc/config.gz" | sha256sum | cut -d' ' -f1)"
  fi
  if gzip -dc "$R/proc/config.gz" | grep -q '^CONFIG_MODULE_SIG_FORCE=y$'; then
    printf 'signed modules: REQUIRED (CONFIG_MODULE_SIG_FORCE=y)\n'
  elif gzip -dc "$R/proc/config.gz" | grep -q '^CONFIG_MODULE_SIG=y$'; then
    printf 'signed modules: checked but not required (CONFIG_MODULE_SIG=y, no CONFIG_MODULE_SIG_FORCE)\n'
  else
    printf 'signed modules: not checked (no CONFIG_MODULE_SIG)\n'
  fi
  gzip -dc "$R/proc/config.gz" | grep -E "$SELECT"
else
  printf '(/proc/config.gz is not readable here)\n'
fi

section 'Devices and the driver bound to each, by bus'
printf 'device  driver=NAME (or none)  module=MODULE (if the driver is a module)  node=DEVICE-TREE PATH  compatible=...\n'
for b in "$R"/sys/bus/*; do
  [ -d "$b/devices" ] || continue
  printf -- '--- %s ---\n' "${b##*/}"
  for d in "$b"/devices/*; do
    [ -e "$d" ] || continue
    line="${d##*/}  driver=none"
    [ -L "$d/driver" ] && line="${d##*/}  driver=$(link_name "$d/driver")"
    m=$(link_name "$d/driver/module")
    [ -n "$m" ] && line="$line  module=$m"
    n=$(node_of "$d")
    [ -n "$n" ] && line="$line  node=$n"
    c=$(compat_of "$d")
    [ -n "$c" ] && line="$line  compatible=$c"
    if [ -r "$d/idVendor" ]; then
      line="$line  id=$(value "$d/idVendor"):$(value "$d/idProduct")"
      [ -r "$d/product" ] && line="$line  product=$(value "$d/product")"
    fi
    printf '%s\n' "$line"
  done
done | redact

section 'Loaded modules (/proc/modules: name, size, use count, used by)'
cut -d' ' -f1-4 "$R/proc/modules" 2>/dev/null || printf '(none, or /proc/modules not readable)\n'

section 'Sound cards and PCM devices'
cat "$R/proc/asound/cards" 2>/dev/null || printf '(no /proc/asound/cards)\n'
cat "$R/proc/asound/pcm" 2>/dev/null
for c in "$R"/sys/class/sound/card*; do
  [ -d "$c" ] || continue
  printf '%s: device driver=%s node=%s compatible=%s\n' "${c##*/}" "$(link_name "$c/device/driver")" \
    "$(node_of "$c/device")" "$(compat_of "$c/device")"
done

section 'Screen: DRM devices and connectors, framebuffers, backlights'
for d in "$R"/sys/class/drm/*; do
  [ -d "$d" ] || continue
  printf '%s' "${d##*/}"
  [ -r "$d/status" ] && printf '  status=%s' "$(value "$d/status")"
  [ -r "$d/enabled" ] && printf '  enabled=%s' "$(value "$d/enabled")"
  [ -r "$d/modes" ] && printf '  mode=%s' "$(value "$d/modes")"
  [ -L "$d/device/driver" ] && printf '  device driver=%s' "$(link_name "$d/device/driver")"
  printf '\n'
done
for f in "$R"/sys/class/graphics/fb*; do
  [ -d "$f" ] || continue
  printf '%s  name=%s  virtual_size=%s  bits_per_pixel=%s  rotate=%s\n' "${f##*/}" "$(value "$f/name")" \
    "$(value "$f/virtual_size")" "$(value "$f/bits_per_pixel")" "$(value "$f/rotate")"
done
for l in "$R"/sys/class/backlight/*; do
  [ -d "$l" ] || continue
  printf '%s  type=%s  max_brightness=%s  brightness=%s  device driver=%s  node=%s  compatible=%s\n' "${l##*/}" \
    "$(value "$l/type")" "$(value "$l/max_brightness")" "$(value "$l/brightness")" \
    "$(link_name "$l/device/driver")" "$(node_of "$l/device")" "$(compat_of "$l/device")"
done

section 'Input devices (/proc/bus/input/devices: id, name, place, handlers)'
if [ -r "$R/proc/bus/input/devices" ]; then
  grep -E '^(I|N|P|H):' "$R/proc/bus/input/devices" | redact
else
  printf '(no /proc/bus/input/devices)\n'
fi

section 'Regulators'
for r in "$R"/sys/class/regulator/regulator.*; do
  [ -r "$r/name" ] || continue
  printf '%s  name=%s  state=%s  microvolts=%s  users=%s  node=%s\n' "${r##*/}" "$(value "$r/name")" \
    "$(value "$r/state")" "$(value "$r/microvolts")" "$(value "$r/num_users")" "$(node_of "$r/device")"
done

section 'Interrupts'
cat "$R/proc/interrupts" 2>/dev/null || printf '(no /proc/interrupts)\n'

section 'debugfs (only if already mounted): GPIO lines in use, devices waiting for a driver'
if grep -q ' /sys/kernel/debug debugfs ' "$R/proc/mounts" 2>/dev/null; then
  cat "$R/sys/kernel/debug/gpio" 2>/dev/null || printf '(gpio unreadable)\n'
  printf -- '--- devices_deferred ---\n'
  cat "$R/sys/kernel/debug/devices_deferred" 2>/dev/null || printf '(unreadable)\n'
else
  printf 'debugfs is not mounted; this script does not mount it\n'
fi

section 'Kernel messages: firmware, failed or deferred probes, LoadPin, modules'
kernel_log | grep -i -E 'firmware|defer|probe.*(fail|error)|failed to|error -[0-9]|loadpin|module|tainted|signature' |
  redact | head -n 300

section 'Build settings, whole (/proc/config.gz)'
if [ "$have_config" = yes ]; then
  gzip -dc "$R/proc/config.gz"
else
  printf '(/proc/config.gz is not readable here)\n'
fi
printf '\nEND OF DRIVER SURVEY\n'
