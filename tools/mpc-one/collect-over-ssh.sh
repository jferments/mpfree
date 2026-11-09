#!/usr/bin/env bash
# Host-side runner for the read-only MPFree MPC One inventory.
# Defaults match the tested ML-tower -> MPC One direct Ethernet link.
# Writes reports ONLY on the workstation; sends a read-only POSIX sh program by stdin.
set -Eeuo pipefail
umask 077

MPC_IP="${MPC_IP:-10.42.0.141}"
MPC_IFACE="${MPC_IFACE:-eno1np0}"
MPC_HOST_IP_CIDR="${MPC_HOST_IP_CIDR:-10.42.0.1/24}"
MPC_EXPECT_MAC="${MPC_EXPECT_MAC:-00:05:95:02:d4:d2}"
MPC_PORT="${MPC_PORT:-2222}"
MPC_USER="${MPC_USER:-root}"
MPC_KEY="${MPC_KEY:-/home/jesse/.ssh/mpc_one_autorun_ed25519}"

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd -P)"
# Only reviewed, repository-local survey scripts and a bounded host-side benchmark are allowed.
# No arbitrary user-supplied script path or remote command is accepted.
case "${1:-}" in
  "") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-inventory.sh"; REPORT_KIND="mpc-one" ;;
  "--detail") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-detail.sh"; REPORT_KIND="mpc-one-detail" ;;
  "--runtime") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-runtime.sh"; REPORT_KIND="mpc-one-runtime" ;;
  "--io") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-io.sh"; REPORT_KIND="mpc-one-io" ;;
  "--netbench") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-netbench" ;;
  *) printf 'ERROR: Unknown mode. Modes: default baseline, --detail, --runtime, --io (passive link/USB), --netbench (opt-in active SSH load).\n' >&2
     exit 1 ;;
esac
[ "$#" -le 1 ] || { printf 'ERROR: Too many arguments\n' >&2; exit 1; }
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
MPC_REPORT_DIR="${MPC_REPORT_DIR:-$REPO_ROOT/research}"

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
command -v ip >/dev/null 2>&1 || die 'iproute2 not installed on workstation'
command -v ping >/dev/null 2>&1 || die 'ping missing on workstation'
command -v ssh >/dev/null 2>&1 || die 'OpenSSH client missing on workstation'
command -v timeout >/dev/null 2>&1 || die 'timeout missing on workstation'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum missing on workstation'
if [[ "${1:-}" == "--netbench" ]]; then
  [[ -r "$SCRIPT_DIR/network-bench.py" ]] || die "Missing host-only network benchmark: $SCRIPT_DIR/network-bench.py"
  command -v python3 >/dev/null 2>&1 || die "python3 is required for --netbench on workstation"
else
  [[ -f "$REMOTE_SCRIPT" && -r "$REMOTE_SCRIPT" ]] || die "Missing read-only inventory script: $REMOTE_SCRIPT"
fi
[[ -f "$MPC_KEY" && -r "$MPC_KEY" ]] || die "SSH private key is missing or unreadable: $MPC_KEY"
[[ -n "$MPC_EXPECT_MAC" && -n "$MPC_HOST_IP_CIDR" ]] || die 'Device identity gates must be supplied'

if ! ip -4 addr show dev "$MPC_IFACE" | grep -Fq "inet $MPC_HOST_IP_CIDR"; then
  die "Expected $MPC_HOST_IP_CIDR not found on $MPC_IFACE"
fi
route="$(ip -4 route get "$MPC_IP" 2>/dev/null)" || die 'No IPv4 route to MPC'
case "$route" in
  *" dev $MPC_IFACE "*) ;;
  *) die "IPv4 route to MPC is not through $MPC_IFACE: $route" ;;
esac
ping -I "$MPC_IFACE" -c 1 -W 2 "$MPC_IP" >/dev/null || die 'MPC does not answer direct-interface ICMP'

# Keep the device identity gate: never silently accept missing neighbor data.
# In Linux, ARP records can be absent/transient despite a successful ping.
# Read both netlink and /proc/net/arp; safely refresh with bounded ICMP retries.
# The parser checks exact IP + interface and finds "lladdr" by name, not $5.
PARSER="$SCRIPT_DIR/lib/neighbor-identity.sh"
[[ -r "$PARSER" ]] || die "Missing neighbor parser: $PARSER"
# shellcheck source=lib/neighbor-identity.sh
source "$PARSER"

found_mac=""
for attempt in 1 2 3 4; do
  neighbor_lines="$(
    {
      ip -4 neigh show dev "$MPC_IFACE" || :
      ip -4 neigh get "$MPC_IP" dev "$MPC_IFACE" || :
    } 2>/dev/null
  )"
  netlink_mac="$(printf '%s\n' "$neighbor_lines" |
    mpfree_mac_from_ip_neigh_lines "$MPC_IP" "$MPC_IFACE" | sort -u)"
  proc_mac=""
  if [[ -r /proc/net/arp ]]; then
    proc_mac="$(mpfree_mac_from_proc_arp_lines "$MPC_IP" "$MPC_IFACE" < /proc/net/arp | sort -u)"
  fi

  if [[ -n "$netlink_mac" && -n "$proc_mac" && "$netlink_mac" != "$proc_mac" ]]; then
    die "Conflicting neighbor identities for $MPC_IP on $MPC_IFACE: ip=$netlink_mac, proc=$proc_mac"
  fi
  found_mac="${netlink_mac:-$proc_mac}"
  if [[ -n "$found_mac" ]]; then
    break
  fi
  if (( attempt < 4 )); then
    printf 'No complete ARP neighbor entry yet (attempt %s/4); refreshing with ICMP...\n' "$attempt" >&2
    sleep 1
    ping -I "$MPC_IFACE" -c 1 -W 2 "$MPC_IP" >/dev/null ||
      die "MPC stopped responding on $MPC_IFACE during identity check"
  fi
done

if [[ -z "$found_mac" ]]; then
  printf 'Diagnostic ip-neigh output:\n' >&2
  ip -4 neigh show dev "$MPC_IFACE" >&2 || true
  printf 'Diagnostic /proc/net/arp record(s):\n' >&2
  if [[ -r /proc/net/arp ]]; then
    awk -v ip="$MPC_IP" -v dev="$MPC_IFACE" 'NR==1 || ($1==ip && $6==dev)' /proc/net/arp >&2
  fi
  die "No complete ARP/MAC identity for $MPC_IP on $MPC_IFACE (not a mismatched MAC). No SSH or MPC commands were run. Do not bypass the identity check."
fi
if [[ ! "$found_mac" =~ ^([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}$ ]]; then
  die "Unexpected MAC format from neighbor tables: $found_mac"
fi
if [[ "$found_mac" != "${MPC_EXPECT_MAC,,}" ]]; then
  die "Device identity mismatch: expected ${MPC_EXPECT_MAC,,}, observed $found_mac"
fi

mkdir -p -- "$MPC_REPORT_DIR"
REPORT="$(mktemp "$MPC_REPORT_DIR/${REPORT_KIND}-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX.txt")"
echo "Verified direct Ethernet: $MPC_IFACE ($MPC_HOST_IP_CIDR) -> $MPC_IP ($found_mac)"
echo "Collecting read-only report to $REPORT"

if [[ "${1:-}" == "--netbench" ]]; then
  echo "ACTIVE BENCHMARK REQUESTED: two 4 MiB SSH data transfers plus 20 ICMP pings."
  echo "Avoid running while recording/playing; it may briefly load the MPC CPU and network."
  if ! timeout 95 python3 "$SCRIPT_DIR/network-bench.py" \
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" \
    --key "$MPC_KEY" --iface "$MPC_IFACE" > "$REPORT" 2>&1; then
    echo "Benchmark failed safely; output retained: $REPORT" >&2
    sed -n '1,50p' "$REPORT" >&2
    exit 1
  fi
else
if ! timeout 180 ssh -T \
   -o BatchMode=yes \
   -o IdentitiesOnly=yes \
   -o StrictHostKeyChecking=yes \
   -o ConnectTimeout=8 \
   -o ForwardAgent=no \
   -o ClearAllForwardings=yes \
   -i "$MPC_KEY" \
   -p "$MPC_PORT" \
   "$MPC_USER@$MPC_IP" 'sh -s' < "$REMOTE_SCRIPT" > "$REPORT" 2>&1; then
  echo "SSH or remote inventory failed; report retained for diagnosis: $REPORT" >&2
  sed -n '1,30p' "$REPORT" >&2
  echo "For a passphrase-protected key, first run: ssh-add $MPC_KEY" >&2
  exit 1
fi
fi

sha256sum "$REPORT" > "$REPORT.sha256"
printf '\nSUCCESS: %s\n' "$REPORT"
printf 'SHA256: '; sha256sum "$REPORT" | awk '{print $1}'
printf 'Review/redact this report before sharing; NEVER commit raw logs or device identifiers.\n'
