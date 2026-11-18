#!/bin/sh
# MPFree USB-B survey - STRICTLY READ-ONLY.
# Streamed over SSH stdin by collect-over-ssh.sh --usb-device, on Akai's system
# with Akai's app in Standalone Mode as usual. Run it twice: once with nothing in
# the USB-B socket, then with a cable from USB-B to a computer
# (docs/dev/hardware/usb-b-midi.md has the round's steps).
#
# Answers what MPFree needs to know before it can be a USB MIDI device for a
# computer: whether the kernel can be a USB MIDI (or audio) device, which gadget
# Akai's system sets up on the USB-B socket and with which functions, which
# program holds it, which start-up files set it up, and what changes when a
# computer is plugged in.
#
# It reads procfs, sysfs, the small attribute files of the USB gadgets already
# set up in configfs, the kernel log and Akai's start-up files. It never
# mounts, never loads or unloads a module, never binds or unbinds a gadget or a
# driver, never creates or removes anything in configfs, never opens a MIDI or
# sound device and never writes a file. It prints no MAC address or serial
# number: a gadget's serial number and network functions' addresses are never
# read, and the kernel log is filtered.
set -u
R="${MPFREE_FAKE_ROOT:-}"
printf 'MPFREE USB-B SURVEY - READ ONLY\n'
printf 'MAC addresses and serial numbers are replaced with <MAC> and <serial>.\n'

# The kernel's build settings for being a USB device (a gadget) and for MIDI.
SELECT='^(# )?CONFIG_(USB_GADGET[A-Z_0-9]*|USB_LIBCOMPOSITE|USB_CONFIGFS[A-Z_0-9]*|USB_F_[A-Z_0-9]+|USB_U_[A-Z_0-9]+|USB_G_[A-Z_0-9]+|USB_MIDI_GADGET|USB_AUDIO|USB_FUNCTIONFS[A-Z_0-9]*|USB_DWC2[A-Z_0-9]*|USB_ROLE_SWITCH|USB_OTG[A-Z_0-9]*|CONFIGFS_FS|SND_RAWMIDI|SND_SEQUENCER|USB_GADGETFS)(=| is not set)'

section() { printf '\n=== %s ===\n' "$1"; }
redact() {
  sed -e 's/[0-9A-Fa-f][0-9A-Fa-f]\(:[0-9A-Fa-f][0-9A-Fa-f]\)\{5\}/<MAC>/g' \
      -e 's/\([A-Za-z_.]*[Ss][Ee][Rr][Ii][Aa][Ll][A-Za-z_.]*[=:] *\)[^ ,]*/\1<serial>/g'
}
value() { [ -r "$1" ] && head -n 1 "$1" 2>/dev/null | redact; return 0; }
link_name() { [ -L "$1" ] && basename "$(readlink "$1")"; return 0; }
show() {
  # show LABEL FILE: one attribute, when the file is there.
  [ -r "$2" ] && printf '  %s: %s\n' "$1" "$(value "$2")"
  return 0
}
kernel_log() { if [ -n "$R" ]; then cat "$R/kernel-log" 2>/dev/null; else dmesg 2>/dev/null; fi; }
strip_root() { printf '%s\n' "${1#"$R"}"; }

section 'Kernel'
printf 'release: %s\n' "$(value "$R/proc/sys/kernel/osrelease")"
printf 'version: %s\n' "$(value "$R/proc/sys/kernel/version")"
printf 'uptime: %s s\n' "$(cut -d' ' -f1 "$R/proc/uptime" 2>/dev/null)"

section 'USB device controllers (/sys/class/udc)'
found=0
for p in "$R"/sys/class/udc/*; do
  [ -e "$p" ] || continue
  found=1
  printf '%s\n' "$(basename "$p")"
  for name in state current_speed maximum_speed is_otg is_a_peripheral a_hnp_support function; do
    show "$name" "$p/$name"
  done
  printf '  driver: %s\n' "$(link_name "$p/device/driver")"
done
[ "$found" -eq 1 ] || printf '(no USB device controller registered)\n'

section 'USB role switches (/sys/class/usb_role)'
found=0
for p in "$R"/sys/class/usb_role/*; do
  [ -e "$p" ] || continue
  found=1
  printf '%s\n' "$(basename "$p")"
  show role "$p/role"
done
[ "$found" -eq 1 ] || printf '(none)\n'

section 'Gadgets set up in configfs (/sys/kernel/config/usb_gadget)'
G="$R/sys/kernel/config/usb_gadget"
if [ -d "$G" ]; then
  found=0
  for g in "$G"/*; do
    [ -d "$g" ] || continue
    found=1
    printf 'gadget: %s\n' "$(basename "$g")"
    udc=$(value "$g/UDC")
    printf '  bound to: %s\n' "${udc:-(nothing: the gadget is set up but not on a socket)}"
    for name in idVendor idProduct bcdDevice bcdUSB bDeviceClass bDeviceSubClass bDeviceProtocol bMaxPacketSize0; do
      show "$name" "$g/$name"
    done
    for s in "$g"/strings/*; do
      [ -d "$s" ] || continue
      # Never the serial number.
      show "strings $(basename "$s") manufacturer" "$s/manufacturer"
      show "strings $(basename "$s") product" "$s/product"
    done
    for c in "$g"/configs/*; do
      [ -d "$c" ] || continue
      printf '  config: %s\n' "$(basename "$c")"
      show '  MaxPower' "$c/MaxPower"
      show '  bmAttributes' "$c/bmAttributes"
      for s in "$c"/strings/*; do
        [ -d "$s" ] || continue
        show "  strings $(basename "$s") configuration" "$s/configuration"
      done
      for l in "$c"/*; do
        [ -L "$l" ] && printf '    uses function: %s\n' "$(link_name "$l")"
      done
    done
    for f in "$g"/functions/*; do
      [ -d "$f" ] || continue
      fn=$(basename "$f")
      printf '  function: %s\n' "$fn"
      case "$fn" in
        midi.*|midi2.*)
          for name in id index in_ports out_ports buflen qlen; do show "  $name" "$f/$name"; done ;;
        uac1.*|uac2.*)
          for name in c_chmask c_srate c_ssize p_chmask p_srate p_ssize req_number function_name; do
            show "  $name" "$f/$name"
          done ;;
        # Network functions hold MAC addresses and the rest is not about MIDI:
        # only their names are printed.
      esac
    done
  done
  [ "$found" -eq 1 ] || printf '(configfs is there, but no gadget is set up in it)\n'
else
  printf '(no usb_gadget directory: configfs is not mounted, or the kernel has no configfs gadgets)\n'
fi

section 'Build settings for being a USB device (/proc/config.gz)'
if [ -r "$R/proc/config.gz" ] && gzip -dc "$R/proc/config.gz" >/dev/null 2>&1; then
  gzip -dc "$R/proc/config.gz" | grep -E "$SELECT"
else
  printf '(/proc/config.gz is not readable here)\n'
fi

section 'Gadget modules: loaded (/proc/modules), then present on disk'
if [ -r "$R/proc/modules" ]; then
  grep -E '^(libcomposite|usb_f_[a-z0-9_]+|g_[a-z0-9_]+|u_audio|u_ether|u_serial|snd_rawmidi|dwc2|configfs) ' "$R/proc/modules" \
    | cut -d' ' -f1,3,4
fi
rel=$(value "$R/proc/sys/kernel/osrelease")
d="$R/lib/modules/$rel/kernel/drivers/usb/gadget"
if [ -n "$rel" ] && [ -d "$d" ]; then
  for f in "$d"/*.ko* "$d"/*/*.ko* "$d"/*/*/*.ko*; do
    [ -e "$f" ] && printf 'on disk: %s\n' "$(strip_root "$f")"
  done
else
  printf '(no %s)\n' "/lib/modules/$rel/kernel/drivers/usb/gadget"
fi

section 'configfs and functionfs'
grep -E 'configfs|functionfs' "$R/proc/filesystems" 2>/dev/null
grep -E ' (configfs|functionfs) ' "$R/proc/mounts" 2>/dev/null

section 'Sound cards (/proc/asound/cards)'
if [ -r "$R/proc/asound/cards" ]; then
  redact < "$R/proc/asound/cards"
else
  printf '(no /proc/asound/cards)\n'
fi

section 'Programs holding a gadget endpoint or a MIDI or sound device'
for p in "$R"/proc/[0-9]*; do
  [ -d "$p/fd" ] || continue
  for fd in "$p"/fd/*; do
    t=$(readlink "$fd" 2>/dev/null) || continue
    case "$t" in
      */usb-ffs/*|*/ffs*/ep*|*functionfs*|*/gadget/*|/dev/snd/midiC*|/dev/snd/pcmC*|/dev/snd/seq|/dev/midi*)
        printf 'pid %s (%s): %s\n' "$(basename "$p")" "$(value "$p/comm")" "$t" ;;
    esac
  done
done

section 'Start-up files that mention the USB device port'
# The names of files that mention the gadget; for a script or a settings file,
# its matching lines too, leaving out any line about a serial number.
for dir in /etc /lib/systemd /usr/lib/systemd /lib/udev /usr/lib/udev /usr/bin /usr/sbin /usr/libexec /opt; do
  [ -d "$R$dir" ] || continue
  grep -rlsE 'usb_gadget|/sys/class/udc|libcomposite|functionfs|usb_f_midi|f_midi' "$R$dir" 2>/dev/null | head -n 40 |
  while read -r f; do
    case "$(head -c 2 "$f" 2>/dev/null)" in
      '#!') text=1 ;;
      *) case "$f" in
           *.service|*.sh|*.rules|*.conf|*.target|*.socket|*.path|"$R"/etc/*) text=1 ;;
           *) text=0 ;;
         esac ;;
    esac
    if [ "$text" -eq 1 ]; then
      printf '%s\n' "--- $(strip_root "$f") ---"
      grep -nE 'usb_gadget|/sys/class/udc|UDC|libcomposite|functionfs|usb_f_|f_midi|idVendor|idProduct|functions/|configs/' "$f" 2>/dev/null \
        | grep -viE 'serial' | head -n 40 | redact
    else
      printf '%s\n' "--- $(strip_root "$f") (a program; not printed) ---"
    fi
  done
done

section 'Kernel log about the USB device port'
kernel_log | grep -iE 'dwc2|gadget|udc|configfs|f_midi|usb_role|otg|ff580000|functionfs' | tail -n 60 | redact

printf '\nEND OF USB-B SURVEY\n'
