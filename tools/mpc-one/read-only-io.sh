#!/bin/sh
# MPFree passive Ethernet + USB capability inventory. Run on MPC via SSH sh -s.
# Reads procfs, sysfs and OPTIONAL ethtool queries; no services or link changes.
set -u

print_file() {
  name=$1
  if [ -r "$name" ]; then
    printf '  %s: ' "$name"
    cat "$name" 2>/dev/null || :
  fi
}
link_target() {
  if [ -L "$1" ]; then
    printf '  %s -> ' "$1"
    readlink "$1" 2>/dev/null || :
  fi
}

printf 'MPFREE MPC ONE — PASSIVE ETHERNET / USB CAPABILITY INVENTORY\n'
printf 'This command does not alter network links, USB settings or internal storage.\n'
printf '\n=== OS network interfaces ===\n'
if command -v ip >/dev/null 2>&1; then
  ip addr 2>&1 || :
  ip route 2>&1 || :
fi
printf '\n=== Wired network interface end0 ===\n'
if [ -d /sys/class/net/end0 ]; then
  for f in operstate carrier speed duplex mtu type addr_assign_type tx_queue_len;
  do print_file "/sys/class/net/end0/$f"; done
  link_target /sys/class/net/end0/device
  link_target /sys/class/net/end0/device/driver
  link_target /sys/class/net/end0/phydev
  for f in /sys/class/net/end0/statistics/rx_bytes \
           /sys/class/net/end0/statistics/tx_bytes \
           /sys/class/net/end0/statistics/rx_packets \
           /sys/class/net/end0/statistics/tx_packets \
           /sys/class/net/end0/statistics/rx_errors \
           /sys/class/net/end0/statistics/tx_errors \
           /sys/class/net/end0/statistics/rx_dropped \
           /sys/class/net/end0/statistics/tx_dropped; do print_file "$f"; done
else
  echo '  end0 not present; do not infer interface identity'
fi
printf '\n=== PHY identifiers (no raw EEPROM dump) ===\n'
for p in /sys/bus/mdio_bus/devices/*; do
  [ -e "$p" ] || continue
  echo "  mdio node: $(basename "$p")"
  for field in phy_id phy_interface; do print_file "$p/$field"; done
  link_target "$p/driver"
done
printf '\n=== ethtool read-only queries, if installed ===\n'
if command -v ethtool >/dev/null 2>&1; then
  ethtool end0 2>&1 || :
  ethtool -i end0 2>&1 || :
  # Avoid ethtool -S dumping extensive or driver-specific sensitive data.
else
  echo '  ethtool not installed; sysfs above is authoritative for current speed when exposed'
fi
printf '\n=== network statistics from procfs ===\n'
if [ -r /proc/net/dev ]; then
  awk 'NR <= 2 || /^[[:space:]]*(end0|lo):/' /proc/net/dev
fi
printf '\n=== USB host-controller sysfs classes ===\n'
if [ -d /sys/bus/platform/drivers ]; then
  for p in /sys/bus/platform/drivers/*ehci* /sys/bus/platform/drivers/*ohci* /sys/bus/platform/drivers/*dwc*; do
    [ -d "$p" ] && echo "  platform driver: $(basename "$p")"
  done
fi
printf '\n=== enumerated USB devices and negotiated speeds (Mbit/s) ===\n'
for p in /sys/bus/usb/devices/*; do
  [ -r "$p/busnum" ] || continue
  echo "--- $(basename "$p") ---"
  for name in busnum devpath speed version idVendor idProduct bcdDevice \
              bDeviceClass bDeviceSubClass bDeviceProtocol bNumConfigurations \
              bNumInterfaces manufacturer product maxchild; do
    print_file "$p/$name"
  done
  link_target "$p/driver"
  # Intentionally omit unique USB serial, port power controls and binary descriptors.
done
printf '\n=== USB interfaces and bound drivers ===\n'
for p in /sys/bus/usb/devices/*:*; do
  [ -r "$p/bInterfaceNumber" ] || continue
  echo "--- $(basename "$p") ---"
  for name in bInterfaceNumber bInterfaceClass bInterfaceSubClass \
              bInterfaceProtocol bNumEndpoints; do print_file "$p/$name"; done
  link_target "$p/driver"
done
printf '\n=== USB gadget/device role capabilities (read-only) ===\n'
if [ -d /sys/class/udc ]; then
  found=0
  for p in /sys/class/udc/*; do
    [ -e "$p" ] || continue
    found=1
    echo "  UDC: $(basename "$p")"
    for name in state current_speed maximum_speed is_otg; do print_file "$p/$name"; done
    link_target "$p/device/driver"
  done
  [ "$found" -eq 1 ] || echo '  no UDC registered (not proof of absent USB device-mode hardware)'
else
  echo '  UDC class unavailable (not proof that USB-B device mode is impossible)'
fi
printf '\n=== USB gadget module names and configfs presence ===\n'
if [ -r /proc/modules ]; then
  awk '/(^| )(libcomposite|usb_f_|g_ether|g_audio|dwc2|dwc3|usb_f_rndis|u_ether)( |$)/ {print $1}' /proc/modules
fi
if [ -d /sys/kernel/config/usb_gadget ]; then
  echo '  USB gadget configfs path exists (no files read/written in it)'
else
  echo '  USB gadget configfs path absent (not proof of kernel capability)'
fi
printf '\nEND MPFREE PASSIVE ETHERNET/USB INVENTORY\n'
