#!/bin/sh
# mpfree hardware survey: READ ONLY.
# Run manually on a device where you are authorized to inspect system information.
# Does not request root, write files, install packages, alter services, or touch firmware.
# Output may contain identifying paths/devices. Review and redact before sharing.
set -u

section() { printf '\n===== %s =====\n' "$1"; }
try_cmd() {
    if command -v "$1" >/dev/null 2>&1; then
        "$@" 2>&1 || true
    else
        printf 'unavailable: %s\n' "$1"
    fi
}
show_file() {
    if [ -r "$1" ]; then
        cat "$1" 2>/dev/null || true
    else
        printf 'unavailable: %s\n' "$1"
    fi
}
section "Kernel and architecture"
try_cmd uname -a
section "Device model"
if [ -r /proc/device-tree/model ]; then
    tr -d '\000' < /proc/device-tree/model
    printf '\n'
else
    printf 'unavailable\n'
fi
section "Memory"
show_file /proc/meminfo
section "ALSA cards"
show_file /proc/asound/cards
section "Playback"
try_cmd aplay -l
section "Capture"
try_cmd arecord -l
section "Raw MIDI"
try_cmd amidi -l
section "ALSA sequencer"
try_cmd aconnect -l
section "Sound device nodes"
if [ -d /dev/snd ]; then ls -l /dev/snd; else printf 'unavailable\n'; fi
section "Input device nodes"
if [ -d /dev/input ]; then ls -l /dev/input; else printf 'unavailable\n'; fi
section "Display device nodes"
if [ -d /dev/dri ]; then ls -l /dev/dri; else printf 'unavailable\n'; fi
if [ -e /dev/fb0 ]; then ls -l /dev/fb0; fi
section "Kernel module names"
if [ -r /proc/modules ]; then cut -d ' ' -f 1 /proc/modules; fi
section "Done"
printf 'Read-only survey complete. Review output before sharing.\n'
