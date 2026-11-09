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
REMOTE_SCRIPT="$SCRIPT_DIR/read-only-inventory.sh"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
MPC_REPORT_DIR="${MPC_REPORT_DIR:-$REPO_ROOT/research}"

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
command -v ip >/dev/null 2>&1 || die 'iproute2 not installed on workstation'
command -v ping >/dev/null 2>&1 || die 'ping missing on workstation'
command -v ssh >/dev/null 2>&1 || die 'OpenSSH client missing on workstation'
command -v timeout >/dev/null 2>&1 || die 'timeout missing on workstation'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum missing on workstation'
[[ -f "$REMOTE_SCRIPT" && -r "$REMOTE_SCRIPT" ]] || die "Missing read-only inventory script: $REMOTE_SCRIPT"
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

found_mac="$(ip neigh show "$MPC_IP" dev "$MPC_IFACE" | awk '{print tolower($5)}')"
if [[ "$found_mac" != "$MPC_EXPECT_MAC" ]]; then
  die "Device identity mismatch: expected $MPC_EXPECT_MAC, observed <$(test -n "$found_mac" && printf '%s' "$found_mac" || printf 'none')>"
fi

mkdir -p -- "$MPC_REPORT_DIR"
REPORT="$(mktemp "$MPC_REPORT_DIR/mpc-one-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX.txt")"
echo "Verified direct Ethernet: $MPC_IFACE ($MPC_HOST_IP_CIDR) -> $MPC_IP ($found_mac)"
echo "Collecting read-only report to $REPORT"

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

sha256sum "$REPORT" > "$REPORT.sha256"
printf '\nSUCCESS: %s\n' "$REPORT"
printf 'SHA256: '; sha256sum "$REPORT" | awk '{print $1}'
printf 'Review/redact this report before sharing; NEVER commit raw logs or device identifiers.\n'
