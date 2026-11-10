#!/bin/sh
# MPFree passive USB capture of the internal MPC One MIDI controller (POSIX sh, BusyBox ash).
#
# What it does: reads the kernel's usbmon TEXT interface for the USB bus that
# carries the internal controller (from /proc/asound/card0/usbbus), streaming
# it to stdout for at most CAPTURE_SECONDS, then stops.
#
# Not strictly read-only, so it needs an explicit owner opt-in in the host wrapper:
# - Opening the usbmon file makes the kernel copy a summary of each USB transfer
#   on that bus into a per-reader buffer. It does NOT create, alter, delay or
#   block USB traffic; if the reader is slow, usbmon drops events instead.
# - Closing the file (when `cat` exits) removes the hook. Nothing persists:
#   no mount, no module load, no file written, no service touched.
# - Requires debugfs to be ALREADY mounted and usbmon to be ALREADY available;
#   otherwise it prints why and exits without doing anything.
set -u
CAPTURE_SECONDS=240

printf 'MPFREE PASSIVE USBMON CAPTURE V1\n'
command -v sleep >/dev/null 2>&1 || { echo '@@ABORT sleep unavailable'; exit 0; }
dfs="$(awk '$3=="debugfs"{print $2; exit}' /proc/mounts 2>/dev/null)"
[ -n "$dfs" ] || { echo '@@ABORT debugfs not mounted (this script never mounts it)'; exit 0; }
busdev="$(cat /proc/asound/card0/usbbus 2>/dev/null)"
case "$busdev" in
  [0-9][0-9][0-9]/[0-9][0-9][0-9]) ;;
  *) echo "@@ABORT unexpected /proc/asound/card0/usbbus: $busdev"; exit 0 ;;
esac
usbid="$(cat /proc/asound/card0/usbid 2>/dev/null)"
[ "$usbid" = "09e8:0046" ] || { echo "@@ABORT card0 is not the MPC One MIDI controller: $usbid"; exit 0; }
bus="${busdev%%/*}"; bus="${bus#0}"; bus="${bus#0}"
dev="${busdev##*/}"
mon="$dfs/usb/usbmon/${bus}u"
[ -r "$mon" ] || { echo "@@ABORT usbmon text interface not available: $mon"; exit 0; }
printf '@@BUS %s DEV %s SOURCE %s SECONDS %s\n' "$bus" "$dev" "$mon" "$CAPTURE_SECONDS"
cat "$mon" &
pid=$!
printf '@@CAPTURE START\n'
sleep "$CAPTURE_SECONDS"
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
printf '@@CAPTURE END\n'
