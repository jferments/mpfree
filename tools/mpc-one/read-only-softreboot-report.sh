R=''
# MPFree soft-reboot report - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --softreboot-report (developer mode, port 22). The R='' line
# is an empty root prefix; only tests change it.
#
# Answers, without changing anything: which system is running now (Akai's or MPFree's), whether this kernel
# can do the userspace-only restart MPFree uses, the state of /run/nextroot, whether MPFree's stick is in
# and whether its root is a valid boot target, the internal storage controllers, and the recent kernel-log
# and journal lines about the restart or the stick. After a restart into MPFree the dev-mode SSH on port 22
# is gone (MPFree's own sshd listens on 2222 by key), so this report is for the power-ons that ended in
# Akai's system, or when MPFree did not come up.
#
# It reads procfs, sysfs, /etc, the journal and, if the stick's log partition is already mounted, its files.
# It NEVER mounts, starts or stops anything, loads a kernel, or writes storage.
set -u
printf 'MPFREE SOFT-REBOOT REPORT - READ ONLY\n'
printf 'Values that identify the unit (serial, MAC, partition UUIDs) are left out where shown.\n'
section() { printf '\n=== %s ===\n' "$1"; }
show() { [ -r "$1" ] && printf '%s: %s\n' "$1" "$(cat "$1")"; return 0; }

section 'Which system is running now'
if [ -e "$R/etc/mpfree-test" ]; then
  printf 'this is MPFree (/etc/mpfree-test is present on the running root)\n'
else
  printf "this is Akai's system (no /etc/mpfree-test on the running root)\n"
fi
printf 'running release: %s\n' "$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)"
printf 'uptime: %s s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"

section 'Can this kernel restart userspace into another root'
verb="soft-reboot"
svc="systemd-$verb.service"
printf 'systemd version: %s\n' "$(systemctl --version 2>/dev/null | sed -n '1p')"
if [ -e "$R/usr/lib/systemd/system/$svc" ] || [ -e "$R/lib/systemd/system/$svc" ]; then
  printf '%s: present (userspace-only restart available)\n' "$svc"
else
  printf '%s: absent\n' "$svc"
fi
[ -e "$R/run/nextroot" ] && printf '/run/nextroot: present (a restart may be staged)\n' || printf '/run/nextroot: absent\n'

section "MPFree's stick"
dev=$(readlink -f "$R/dev/disk/by-partlabel/mpfree-test-root" 2>/dev/null)
if [ -n "$dev" ] && [ -e "$dev" ]; then
  name=${dev##*/}
  printf 'stick in: yes (/dev/%s)\n' "$name"
  # Where Akai's system has it mounted (read-only inspection there; we never mount it ourselves).
  at=''
  while read -r d m rest; do [ "$d" = "/dev/$name" ] && [ -z "$at" ] && at=$m; done < "$R/proc/mounts"
  if [ -n "$at" ]; then
    printf 'partition 1 mounted by the system at %s; checking its root tree there:\n' "$at"
    root="$R$at"
    [ -x "$root/sbin/init" ] || [ -x "$root/lib/systemd/systemd" ] || [ -x "$root/usr/lib/systemd/systemd" ] &&
      printf '  init: present\n' || printf '  init: MISSING\n'
    [ -e "$root/etc/mpfree-test" ] && printf '  /etc/mpfree-test marker: present\n' || printf '  /etc/mpfree-test marker: MISSING\n'
    for u in mpfree-test-splash.service mpfree-test-app.service mpfree-test-screen.service; do
      [ -e "$root/etc/systemd/system/$u" ] && printf '  startup service: %s\n' "$u" && break
    done
    rel=$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)
    [ -d "$root/lib/modules/$rel" ] && printf '  modules for the running kernel (%s): present\n' "$rel" ||
      printf '  modules for the running kernel (%s): MISSING\n' "$rel"
    for u in acvs.service; do
      m="$root/etc/systemd/system/$u"
      [ -L "$m" ] && [ "$(readlink "$m" 2>/dev/null)" = /dev/null ] && printf "  Akai's app (%s): masked\n" "$u" ||
        printf "  Akai's app (%s): NOT masked\n" "$u"
    done
  else
    printf 'partition 1 is not mounted by the system right now; not inspected (this report never mounts)\n'
  fi
else
  printf 'stick in: no (no partition named mpfree-test-root)\n'
fi

section 'Internal storage still visible (soft-reboot keeps the running device tree)'
blk=''
for b in "$R"/sys/block/mmcblk[0-9]; do [ -d "$b" ] && blk="$blk ${b##*/}"; done
printf 'eMMC/SD block devices:%s\n' "${blk:- none}"
printf 'mounted internal partitions:\n'
grep -E ' (/media/az01-internal|/media/acvs-content|/data) ' "$R/proc/mounts" 2>/dev/null || printf '  (none)\n'

section 'MPFree log partition, if the system has it mounted (read-only)'
logat=''
while read -r d m rest; do
  case "$m" in */MPFREE-LOG) logat=$m ;; esac
done < "$R/proc/mounts"
if [ -n "$logat" ] && [ -d "$R$logat" ]; then
  printf 'MPFREE-LOG at %s; newest files:\n' "$logat"
  ls -t "$R$logat" 2>/dev/null | head -n 10 | sed 's/^/  /'
else
  printf 'not mounted (nothing to read without mounting, which this report never does)\n'
fi

section 'Kernel log and journal lines about the restart or the stick'
dmesg 2>/dev/null | grep -iE 'nextroot|mpfree|loadpin|usb-storage|sd [0-9]' | tail -n 20 || printf '(none)\n'
printf '--- journal (this boot) ---\n'
journalctl -b 0 --no-pager 2>/dev/null | grep -iE "nextroot|$verb|mpfree|Started .*MPC Application|acvs" | tail -n 20 ||
  printf '(journal unavailable)\n'

printf '\nEND OF SOFT-REBOOT REPORT\n'
