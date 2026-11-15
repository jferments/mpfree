R=''
# MPFree boot hook report - STRICTLY READ-ONLY.
# (The R='' line is an empty root prefix; only tests change it.)
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-report` (developer mode, port 22).
#
# On Akai's system, after a power-on: is MPFree's boot hook (hook-install.sh) installed, did it run at
# this power-on and what did it decide (the stick was not in; or it stopped, and why), from its lines in
# the kernel log and /run, and this boot's timeline from the journal. After a soft-reboot that worked, MPFree
# is running instead, so this report is for the power-ons that ended in Akai's system (a soft-reboot keeps
# the kernel and /run, so its lines stay readable if Akai's system came back). It only reads.
set -u
section() { printf '\n=== %s ===\n' "$1"; }
printf 'MPFREE BOOT HOOK REPORT - READ ONLY\n'
SCRIPT=/data/mpfree-hook/hook.sh
RULE=/data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules
CONF=/data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf

section 'This boot'
printf 'release: %s\n' "$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)"
printf 'uptime (s): %s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"
printf 'acvs.service (Akai app): %s\n' "$(systemctl is-active acvs.service 2>/dev/null)"
if [ -e "$R/run/nextroot" ]; then
  printf '/run/nextroot: %s\n' "$(ls -A "$R/run/nextroot" 2>/dev/null | head -n 5 | tr '\n' ' ')"
else
  printf '/run/nextroot: none (nothing staged for a soft-reboot)\n'
fi
[ -e "$R/run/mpfree-hook/soft-reboot" ] && printf 'the hook asked for a soft-reboot at %s s\n' "$(cat "$R/run/mpfree-hook/soft-reboot")"
if [ -e "$R/dev/disk/by-partlabel/mpfree-test-root" ]; then
  printf "MPFree's stick in now: yes (%s)\n" "$(readlink -f "$R/dev/disk/by-partlabel/mpfree-test-root" | sed "s|^$R||")"
else
  printf "MPFree's stick in now: no\n"
fi
# Was the hook there when this boot started? (Its files' change times against the time of power-on.)
up=$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null | cut -d. -f1)
now=$(date +%s 2>/dev/null)
newest=0
for f in "$SCRIPT" "$RULE" "$CONF"; do
  t=$(stat -c %Y "$R$f" 2>/dev/null) || continue
  [ "${t:-0}" -gt "$newest" ] && newest=$t
done
command -v stat >/dev/null 2>&1 || now=''
case "$up$now" in
  *[!0-9]*|'') printf 'hook installed before this boot: cannot tell\n' ;;
  *) if [ "$newest" = 0 ]; then
       printf 'hook installed before this boot: no hook files\n'
     elif [ "$newest" -gt $((now - up)) ]; then
       printf 'hook installed before this boot: NO, %s s after power-on.\n' "$((newest - now + up))"
       printf 'NEXT: switch the MPC off and on, then run this report again.\n'
     else
       printf 'hook installed before this boot: yes\n'
     fi ;;
esac

section 'Hook files'
for f in "$SCRIPT" "$RULE" "$CONF"; do
  if [ -f "$R$f" ]; then printf '%s  %s\n' "$(sha256sum < "$R$f" | cut -d' ' -f1)" "$f"; else printf 'missing: %s\n' "$f"; fi
done
for f in /etc/udev/rules.d/99-mpfree-hook.rules /etc/modprobe.d/99-mpfree-hook.conf; do
  if [ -f "$R$f" ]; then printf 'visible: %s\n' "$f"; else printf 'not visible: %s\n' "$f"; fi
done
for f in /data/mpfree-hook/canary.sh /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules \
         /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf; do
  [ -e "$R$f" ] && printf "the canary's file is still there: %s\n" "$f"
done

section 'What the hook did at this power-on (/run/mpfree-hook.txt)'
head -n 100 "$R/run/mpfree-hook.txt" 2>/dev/null || printf '(nothing: it did not run, or found no reason to note anything)\n'

section 'Hook lines in the kernel log (seconds since power-on)'
dmesg 2>/dev/null | grep 'mpfree-hook' || printf '(none)\n'

section 'Timeline of this boot from the journal (seconds since power-on)'
journalctl -b --no-pager -o short-monotonic 2>/dev/null |
  grep -E 'mpfree-hook|mpfree-jump|MPFree|Mounted .*/data|Writable /etc|Attached SCSI|cfg80211|Connection service|MPC Application|touch-fw|xmos|usbsata|soft-reboot|Reboot System Userspace|nextroot|LoadPin|edisksd.*( 8:[0-9]+ at |add device)' |
  head -n 120

section 'The boot before this one, if the journal keeps it (seconds since that power-on)'
prev=$(journalctl -b -1 --no-pager -o short-monotonic 2>/dev/null |
  grep -E 'mpfree-hook|mpfree-jump|MPFree|soft-reboot|Reboot System Userspace|Connection service|MPC Application' | head -n 60)
if [ -n "$prev" ]; then printf '%s\n' "$prev"; else printf '(none: the journal keeps no earlier boot here, or nothing in it matched)\n'; fi

section 'Rule and modprobe folders'
for d in /etc/udev/rules.d /etc/modprobe.d; do
  [ -d "$R$d" ] && printf '%s: %s\n' "$d" "$(ls "$R$d" | tr '\n' ' ')"
done

printf '\nEND OF BOOT HOOK REPORT\n'
