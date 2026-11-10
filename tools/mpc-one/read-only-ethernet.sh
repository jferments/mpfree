#!/bin/sh
# MPFree Ethernet details - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --ethernet (on the running
# stock system), and run by the kexec test build to record the same details on
# the USB stick, so the two can be compared line by line.
#
# Answers: was the Ethernet PHY found, and is the kernel holding it in reset;
# which clocks feed the Ethernet controller and are they running; how are the
# PHY's reset and interrupt pins set; and what did the kernel say about the
# controller, its MDIO bus and the PHY.
#
# It reads procfs, sysfs, debugfs (only if it is already mounted) and the
# kernel log, and asks ethtool (only if installed) for its query output. It
# never mounts, never changes an interface or a service, never writes a file or
# a register, and never prints a MAC address.
set -u
R="${MPFREE_FAKE_ROOT:-}"
printf 'MPFREE ETHERNET DETAILS - READ ONLY\n'
printf 'MAC addresses are replaced with <MAC>.\n'

section() { printf '\n=== %s ===\n' "$1"; }
redact() { sed -e 's/[0-9A-Fa-f][0-9A-Fa-f]\(:[0-9A-Fa-f][0-9A-Fa-f]\)\{5\}/<MAC>/g'; }
value() { [ -r "$1" ] && cat "$1" 2>/dev/null | head -n 1; return 0; }
show() { [ -r "$1" ] && printf '  %s: %s\n' "$2" "$(value "$1")"; return 0; }
link_name() { [ -L "$1" ] && basename "$(readlink "$1")"; return 0; }
# The tests run this against a fake tree, whose kernel log is a file in it.
kernel_log() { if [ -n "$R" ]; then cat "$R/kernel-log" 2>/dev/null; else dmesg 2>/dev/null; fi; }

section 'Kernel'
printf 'release: %s\n' "$(value "$R/proc/sys/kernel/osrelease")"
printf 'uptime: %s s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"

section 'Network interfaces (not lo)'
for d in "$R"/sys/class/net/*; do
  [ -d "$d" ] || continue
  name=${d##*/}
  [ "$name" = lo ] && continue
  printf '%s\n' "$name"
  show "$d/operstate" operstate
  show "$d/flags" flags
  show "$d/carrier" carrier
  show "$d/mtu" mtu
  printf '  driver: %s\n' "$(link_name "$d/device/driver")"
  if [ -L "$d/phydev" ]; then
    printf '  phy: %s\n' "$(link_name "$d/phydev")"
    show "$d/phydev/phy_id" '  phy_id'
    show "$d/phydev/phy_interface" '  phy_interface'
    printf '    phy driver: %s\n' "$(link_name "$d/phydev/driver")"
  else
    printf '  phy: none attached\n'
  fi
done

section 'MDIO buses'
for b in "$R"/sys/class/mdio_bus/*; do
  [ -d "$b" ] || continue
  printf '%s\n' "${b##*/}"
  for f in transfers errors reads writes transfers_1 errors_1 reads_1 writes_1; do
    show "$b/statistics/$f" "$f"
  done
done

section 'MDIO devices'
for m in "$R"/sys/bus/mdio_bus/devices/*; do
  [ -d "$m" ] || continue
  printf '%s\n' "${m##*/}"
  show "$m/phy_id" phy_id
  show "$m/phy_interface" phy_interface
  show "$m/phy_has_fixups" phy_has_fixups
  printf '  driver: %s\n' "$(link_name "$m/driver")"
  printf '  attached to: %s\n' "$(link_name "$m/attached_dev")"
done

section 'Interrupts of the Ethernet controller and PHY'
grep -i -E 'eth[0-9]|end[0-9]|stmmac|mdio|ff290000|macirq' "$R/proc/interrupts" 2>/dev/null || printf '(none listed)\n'

section 'PHY supply (VCCIO_PMU) and other Ethernet regulators'
for r in "$R"/sys/class/regulator/regulator.*; do
  [ -r "$r/name" ] || continue
  rname=$(value "$r/name")
  case "$rname" in
    VCCIO_PMU|*[Gg][Mm][Aa][Cc]*|*[Ll][Aa][Nn]*|*[Pp][Hh][Yy]*)
      printf '%s (%s)\n' "$rname" "${r##*/}"
      show "$r/state" state
      show "$r/microvolts" microvolts
      show "$r/num_users" num_users
      ;;
  esac
done

section 'debugfs'
if grep -q ' /sys/kernel/debug debugfs ' "$R/proc/mounts" 2>/dev/null; then
  printf 'mounted at /sys/kernel/debug\n'

  section 'GPIO lines in use (debugfs gpio; the PHY reset is gpio4 B0, its interrupt gpio3 B7)'
  cat "$R/sys/kernel/debug/gpio" 2>/dev/null || printf '(unreadable)\n'

  section 'Clocks of the Ethernet controller (debugfs clk_summary)'
  if [ -r "$R/sys/kernel/debug/clk/clk_summary" ]; then
    head -n 3 "$R/sys/kernel/debug/clk/clk_summary"
    grep -E 'mac|gmac|_peri' "$R/sys/kernel/debug/clk/clk_summary"
  else
    printf '(unreadable)\n'
  fi

  section 'Ethernet pins (debugfs pinctrl)'
  for p in "$R"/sys/kernel/debug/pinctrl/*; do
    [ -r "$p/pinmux-pins" ] || continue
    printf '%s pinmux-pins:\n' "${p##*/}"
    grep -E 'ff290000|\(gpio4-8\)|\(gpio3-15\)' "$p/pinmux-pins" 2>/dev/null
    if [ -r "$p/pinconf-pins" ]; then
      printf '%s pinconf-pins:\n' "${p##*/}"
      grep -E '\(gpio4-8\)|\(gpio3-15\)' "$p/pinconf-pins" 2>/dev/null
    fi
  done

  section 'Devices waiting for a driver (debugfs devices_deferred)'
  cat "$R/sys/kernel/debug/devices_deferred" 2>/dev/null || printf '(unreadable)\n'
else
  printf 'debugfs is not mounted; this script does not mount it\n'
fi

section 'Kernel messages about the Ethernet controller, MDIO bus and PHY'
kernel_log | grep -i -E 'ff290000|stmmac|dwmac|mdio|rtl82|eth0|end0' | redact

section 'ethtool (queries only)'
if command -v ethtool >/dev/null 2>&1; then
  for d in "$R"/sys/class/net/*; do
    name=${d##*/}
    [ "$name" = lo ] && continue
    [ -d "$d" ] || continue
    printf -- '--- ethtool -i %s ---\n' "$name"
    ethtool -i "$name" 2>&1 | redact
    printf -- '--- ethtool %s ---\n' "$name"
    ethtool "$name" 2>&1 | redact
  done
else
  printf 'ethtool is not installed\n'
fi
printf '\nEND OF ETHERNET DETAILS\n'
