R=''
# MPFree boot-hook canary report - STRICTLY READ-ONLY.
# (The R='' line is an empty root prefix; only tests change it.)
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary-report` (developer mode, port 22).
#
# After a power-on with a USB stick in, it shows whether the canary that hook-canary-install.sh put in
# Akai's writable /etc layer ran, and when: the canary's lines from the kernel log and /run (from the
# udev rule, the modprobe line for cfg80211 and the short services they started), and this boot's
# timeline from the journal (when /data and /etc were mounted, when the stick appeared, when the
# canary's services and Akai's app were started). It only reads.
set -u
section() { printf '\n=== %s ===\n' "$1"; }
printf 'MPFREE BOOT-HOOK CANARY REPORT - READ ONLY\n'

section 'This boot'
printf 'release: %s\n' "$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)"
printf 'uptime (s): %s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"
printf 'acvs.service (Akai app): %s\n' "$(systemctl is-active acvs.service 2>/dev/null)"
printf "kernel's module loader: %s\n" "$(cat "$R/proc/sys/kernel/modprobe" 2>/dev/null)"
printf 'cfg80211 loaded: %s\n' "$(grep -q '^cfg80211 ' "$R/proc/modules" 2>/dev/null && echo yes || echo no)"
# Was the canary there when this boot started? (Its files' change times against the time of power-on.)
up=$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null | cut -d. -f1)
now=$(date +%s 2>/dev/null)
newest=0
for f in /data/mpfree-hook/canary.sh /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules \
         /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf; do
  t=$(stat -c %Y "$R$f" 2>/dev/null) || continue
  [ "${t:-0}" -gt "$newest" ] && newest=$t
done
command -v stat >/dev/null 2>&1 || now=''
case "$up$now" in
  *[!0-9]*|'') printf 'canary installed before this boot: cannot tell\n' ;;
  *) if [ "$newest" -gt $((now - up)) ]; then
       printf 'canary installed before this boot: NO, %s s after power-on.\n' "$((newest - now + up))"
       printf 'NEXT: switch the MPC off and on (stick in), wait for the main screen, then run this report again.\n'
     else
       printf 'canary installed before this boot: yes\n'
     fi ;;
esac

section 'Canary files'
for f in /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules \
         /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf /data/mpfree-hook/canary.sh; do
  if [ -f "$R$f" ]; then printf '%s  %s\n' "$(sha256sum < "$R$f" | cut -d' ' -f1)" "$f"; else printf 'missing: %s\n' "$f"; fi
done
for f in /etc/udev/rules.d/99-mpfree-canary.rules /etc/modprobe.d/99-mpfree-canary.conf; do
  if [ -f "$R$f" ]; then printf 'visible: %s\n' "$f"; else printf 'not visible: %s\n' "$f"; fi
done

section 'Canary lines in the kernel log (seconds since power-on)'
dmesg 2>/dev/null | grep 'mpfree-canary' || printf '(none)\n'

section '/run/mpfree-canary.txt'
head -n 300 "$R/run/mpfree-canary.txt" 2>/dev/null || printf '(none)\n'

section 'Timeline of this boot from the journal (seconds since power-on)'
journalctl -b --no-pager -o short-monotonic 2>/dev/null |
  grep -E 'mpfree-canary|MPFree canary|Mounted .*/data|Writable /etc|systemd-udevd|udev-trigger|Coldplug|Attached SCSI|usb-storage|sd [0-9]+:|\[sd[a-z]\]|cfg80211|Connection service|MPC Application|script runner|edisksd.*( 8:[0-9]+ at |add device)|MPFREE' |
  head -n 120

section 'Rule and modprobe folders'
for d in /etc/udev/rules.d /run/udev/rules.d /etc/modprobe.d; do
  [ -d "$R$d" ] && printf '%s: %s\n' "$d" "$(ls "$R$d" | tr '\n' ' ')"
done

printf '\nEND OF BOOT-HOOK CANARY REPORT\n'
