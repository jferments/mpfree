#!/usr/bin/env bash
# Pure parsers for Linux neighbor output. No network access or writes here.
# Usage: printf '%s\n' "$lines" | mpfree_mac_from_ip_neigh_lines IP IFACE
mpfree_mac_from_ip_neigh_lines() {
  local address="$1" device="$2"
  awk -v target="$address" -v interface="$device" '
    $1 == target {
      on_device = 0
      mac = ""
      for (i = 2; i < NF; ++i) {
        if ($i == "dev" && $(i+1) == interface) on_device = 1
        if ($i == "lladdr") mac = $(i+1)
      }
      if (on_device && mac != "") print tolower(mac)
    }'
}
# /proc/net/arp columns: IP HWtype Flags HWaddr Mask Device.
# Only include complete (0x2) ARP records; FAILED / incomplete do not count.
mpfree_mac_from_proc_arp_lines() {
  local address="$1" device="$2"
  awk -v target="$address" -v interface="$device" '
    NR > 1 && $1 == target && $6 == interface &&
    $3 ~ /^0[xX][0-9A-Fa-f]+$/ {
      # Linux ARP flags are a hexadecimal bitmask, where 0x2 means complete.
      flags = tolower($3)
      if (flags == "0x2" || flags == "0x6") print tolower($4)
    }'
}
