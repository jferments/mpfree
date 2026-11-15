R=''
# MPFree boot-hook canary report - STRICTLY READ-ONLY.
# (The R='' line is an empty root prefix; only tests change it.)
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary-report` (developer mode, port 22).
#
# After a power-on with a USB stick in, it shows whether the canary rule that
# hook-canary-install.sh put in Akai's writable /etc layer ran, and when: the
# canary's lines from the kernel log and /run, and this boot's timeline from
# the journal (when /data and /etc were mounted, when the stick appeared,
# when udev ran the rule, when Akai's app was started). It only reads.
set -u
section() { printf '\n=== %s ===\n' "$1"; }
printf 'MPFREE BOOT-HOOK CANARY REPORT - READ ONLY\n'

section 'This boot'
printf 'release: %s\n' "$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)"
printf 'uptime (s): %s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"
printf 'acvs.service (Akai app): %s\n' "$(systemctl is-active acvs.service 2>/dev/null)"

section 'Canary files'
for f in /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules /data/mpfree-hook/canary.sh; do
  if [ -f "$R$f" ]; then printf '%s  %s\n' "$(sha256sum < "$R$f" | cut -d' ' -f1)" "$f"; else printf 'missing: %s\n' "$f"; fi
done
if [ -f "$R/etc/udev/rules.d/99-mpfree-canary.rules" ]; then
  printf 'the rule is visible in /etc/udev/rules.d\n'
else
  printf 'the rule is not visible in /etc/udev/rules.d\n'
fi

section 'Canary lines in the kernel log (seconds since power-on)'
dmesg 2>/dev/null | grep 'mpfree-canary' || printf '(none)\n'

section '/run/mpfree-canary.txt'
head -n 300 "$R/run/mpfree-canary.txt" 2>/dev/null || printf '(none)\n'

section 'Timeline of this boot from the journal (seconds since power-on)'
journalctl -b --no-pager -o short-monotonic 2>/dev/null |
  grep -E 'mpfree-canary|Mounted .*/data|Writable /etc|systemd-udevd|udev-trigger|Coldplug|Attached SCSI|usb-storage|sd [0-9]+:|\[sd[a-z]\]|MPC Application|script runner|edisks|MPFREE' |
  head -n 120

section 'udev rule folders'
for d in /etc/udev/rules.d /run/udev/rules.d; do
  [ -d "$R$d" ] && printf '%s: %s\n' "$d" "$(ls "$R$d" | tr '\n' ' ')"
done

printf '\nEND OF BOOT-HOOK CANARY REPORT\n'
