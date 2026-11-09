#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=../lib/neighbor-identity.sh
source "$HERE/../lib/neighbor-identity.sh"

expect_equal() {
  local actual="$1" expected="$2" label="$3"
  [[ "$actual" == "$expected" ]] || {
    printf 'FAIL: %s: got <%s>, expected <%s>\n' "$label" "$actual" "$expected" >&2
    return 1
  }
  printf 'PASS: %s\n' "$label"
}
ip=10.42.0.141
iface=eno1np0
mac=00:05:95:02:d4:d2
sample_neighbor=$'10.42.0.141 dev eno1np0 lladdr 00:05:95:02:D4:D2 REACHABLE\n10.42.0.142 dev eno1np0 lladdr de:ad:be:ef:00:01 STALE\n10.42.0.141 dev wlp228s0 lladdr 11:22:33:44:55:66 REACHABLE'
expect_equal "$(printf '%s\n' "$sample_neighbor" | mpfree_mac_from_ip_neigh_lines "$ip" "$iface")" "$mac" 'named lladdr, expected NIC and lowercase MAC'

no_mac_neighbor=$'10.42.0.141 dev eno1np0 INCOMPLETE\n10.42.0.142 dev eno1np0 lladdr de:ad:be:ef:00:01 REACHABLE'
expect_equal "$(printf '%s\n' "$no_mac_neighbor" | mpfree_mac_from_ip_neigh_lines "$ip" "$iface")" "" 'no invented MAC on incomplete entry'

wrong_nic='10.42.0.141 dev wlp228s0 lladdr 11:22:33:44:55:66 STALE'
expect_equal "$(printf '%s\n' "$wrong_nic" | mpfree_mac_from_ip_neigh_lines "$ip" "$iface")" "" 'reject wrong interface'

proc_sample=$'IP address       HW type     Flags       HW address            Mask     Device\n10.42.0.141     0x1         0x2         00:05:95:02:D4:D2     *        eno1np0\n10.42.0.141     0x1         0x0         11:22:33:44:55:66     *        eno1np0\n10.42.0.141     0x1         0x2         11:22:33:44:55:66     *        wlp228s0'
expect_equal "$(printf '%s\n' "$proc_sample" | mpfree_mac_from_proc_arp_lines "$ip" "$iface")" "$mac" 'complete /proc ARP entry only'

proc_none=$'IP address       HW type     Flags       HW address            Mask     Device\n10.42.0.141     0x1         0x0         00:00:00:00:00:00     *        eno1np0'
expect_equal "$(printf '%s\n' "$proc_none" | mpfree_mac_from_proc_arp_lines "$ip" "$iface")" "" 'reject incomplete /proc ARP entry'
printf 'PASS: all MAC parser tests passed\n'
