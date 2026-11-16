#!/bin/sh
# MPFree setup, run once by Akai's script runner (developer mode on, the MPC asks "run autorun.sh?" and
# you tap Yes). It is on the MPFREE partition of the stick the MPFree USB creator made.
#
# mode.txt next to this file says what it does:
#   install  checks that this is an MPC One running the Akai release the stick was made for, with the stick
#            in, then runs hook-install.sh (MPFree's startup hook: three small files on the MPC's data
#            partition, nothing else), then switches developer mode off and restarts the MPC (below). With the
#            stick in at power-on, the MPC then starts MPFree from the stick; without it, Akai's system starts
#            as before.
#   remove   runs hook-remove.sh (deletes those three files and the folders made for them), writes "removed"
#            in mode.txt, then switches developer mode off and restarts.
#   removed  does nothing and does not restart: the hook was already removed with this stick (so tapping Yes
#            again after the restart is harmless). The creator's "Set it back to install" undoes it.
# Developer mode off: after install or remove, the restart is Akai's own "developer mode off" script,
# /usr/lib/az0x/user-access/disable.sh, which deletes /data/root and /data/ssh (developer mode's root login,
# its SSH settings and any keys added to them) and restarts, so the MPC is not left with root SSH and no
# password. It is skipped, and the restart is a plain one, when the stick carries developer tools (an
# mpfree-dev folder next to this file) or a file named keep-developer-mode, or when the script is missing.
# Undo: switch developer mode on again in Akai's menu (SHIFT + MENU > Developer).
# Everything it prints goes to setup-log.txt next to this file (or /run/mpfree-setup-log.txt if the drive
# cannot be written). It never writes the MPC's root filesystem, boot partitions or U-Boot environment.
# Undo of install: put "remove" in mode.txt and run it again, or simply leave the stick out at power-on.
R=''
# (The R='' line above is an empty root prefix; only tests change it.)
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
RELEASE='@RELEASE@'
DEVMODE_OFF=/usr/lib/az0x/user-access/disable.sh
here=$(cd "$(dirname "$0")" 2>/dev/null && pwd)
log="$here/setup-log.txt"
( : >> "$log" ) 2>/dev/null || log="$R/run/mpfree-setup-log.txt"
main() {
  echo "MPFree setup, $(date -u 2>/dev/null)"
  echo "from: $here"
  mode=$(tr -cd 'a-z' < "$here/mode.txt" 2>/dev/null)
  echo "mode: ${mode:-?}"
  [ "$(id -u)" = 0 ] || { echo "STOP: this must run as root (Akai's script runner runs it as root)"; return 1; }
  if ! tr '\000' '\n' < "$R/proc/device-tree/compatible" 2>/dev/null | grep -qx 'inmusic,acva'; then
    echo "STOP: this is not an MPC One (inmusic,acva). MPFree has only been tested on the MPC One (Gen 1)."
    return 1
  fi
  echo "board: MPC One (inmusic,acva)"
  rel=$(cat "$R/proc/sys/kernel/osrelease")
  echo "running release: $rel"
  # MPFree's hook mounts the stick at /run/nextroot only for the moment of its jump; anything there now
  # would turn the restart below into a soft-reboot into it.
  case "$mode" in
    install|remove)
      [ ! -e "$R/run/nextroot" ] || {
        echo "STOP: /run/nextroot exists, so a restart now would not be a plain one. Switch the MPC off and on, then try again."
        return 1
      } ;;
  esac
  case "$mode" in
    install)
      [ "$rel" = "$RELEASE" ] || {
        echo "STOP: this stick was made for kernel $RELEASE (Akai 3.10.1); the MPC runs $rel."
        echo "Update the MPC to the firmware the stick was made from, or make the stick again from your MPC's firmware."
        return 1
      }
      [ -e "$R/dev/disk/by-partlabel/mpfree-test-root" ] || {
        echo "STOP: MPFree's stick is not in (no partition named mpfree-test-root)."
        return 1
      }
      for p in dfu-util touch-fw-upd xmosdfu midifirmup az01-update; do
        pidof "$p" > /dev/null 2>&1 && { echo "STOP: $p is running (a firmware update?); try again later."; return 1; }
      done
      sh -c "$(sed "1s|^R=''\$|R='$R'|" "$here/hook-install.sh")" || { echo "STOP: the hook was not installed (see above). Nothing else changed."; return 1; }
      echo "RESULT: MPFree's startup hook is installed. Restarting the MPC in 5 s; leave the stick in."
      return 0 ;;
    remove)
      sh -c "$(sed "1s|^R=''\$|R='$R'|" "$here/hook-remove.sh")" || { echo "STOP: the hook was not removed (see above)."; return 1; }
      echo removed > "$here/mode.txt" 2>/dev/null || echo "(mode.txt could not be set to removed: tap No if the MPC asks again)"
      echo "RESULT: MPFree's startup hook is removed. Restarting the MPC in 5 s."
      return 0 ;;
    removed)
      echo "DONE: nothing to do. MPFree's startup hook was already removed with this stick; the MPC is not restarted."
      echo "To install it again, choose Set it back to install in the MPFree USB creator."
      return 3 ;;
    *)
      echo "STOP: mode.txt must say install or remove."
      return 1 ;;
  esac
}
# The restart, with developer mode switched off first unless the stick asks to keep it (see the top).
finish() {
  sleep 5
  if [ -d "$here/mpfree-dev" ] || [ -e "$here/keep-developer-mode" ]; then
    echo "developer mode stays on: this stick has developer tools (mpfree-dev) or keep-developer-mode. Restarting."
  elif [ -f "$R$DEVMODE_OFF" ]; then
    echo "switching developer mode off with Akai's $DEVMODE_OFF, which restarts the MPC"
    sync
    if [ -x "$R$DEVMODE_OFF" ]; then "$R$DEVMODE_OFF"; else sh "$R$DEVMODE_OFF"; fi
    echo "$DEVMODE_OFF ended (exit $?); restarting in 20 s if it has not already"
    sync
    sleep 20
  else
    echo "Akai's $DEVMODE_OFF is missing, so developer mode stays on. Restarting."
  fi
  sync
  systemctl reboot
}
main >> "$log" 2>&1
rc=$?
sync
case $rc in
  0) finish >> "$log" 2>&1 ;;
  3) exit 0 ;;
  *) exit 1 ;;
esac
