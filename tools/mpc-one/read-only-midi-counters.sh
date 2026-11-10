#!/bin/sh
# MPFree read-only MIDI activity snapshot (POSIX sh, BusyBox ash).
# Prints the kernel's cumulative RawMIDI and USB-MIDI byte/message counters once.
# Reads two procfs text files only: never opens /dev/snd/*, never sends MIDI,
# writes nothing on the MPC. Used by host_midi_activity.py once per phase edge.
set -u
printf 'MPFREE MIDI COUNTERS V1\n'
for f in /proc/asound/card0/midi0 /proc/asound/card0/usbmidi; do
  printf '@@FILE %s\n' "$f"
  if [ -r "$f" ]; then cat "$f" 2>/dev/null || :; else printf '@@UNAVAILABLE\n'; fi
done
printf '@@END\n'
