#!/bin/sh
# MPC One read-only touchscreen, panel, DRM and GPU research.
# Does NOT open evdev, I2C /dev nodes, framebuffer or DRM. No event capture,
# EVIOCGRAB, ioctl, modeset, backlight write, touch calibration or reset.
set -u
printf 'MPFREE TOUCHSCREEN AND DISPLAY HARDWARE MAP - READ ONLY\n'
printf 'NO input event stream consumed, no I2C transactions or DRM resources acquired.\n'

read_property() {
    if [ -r "$1" ]; then
        printf '%s: ' "$1"
        cat "$1" 2>/dev/null || :
        printf '\n'
    fi
}
read_link() {
    if [ -L "$1" ]; then
        printf '%s -> ' "$1"
        readlink "$1" 2>/dev/null || :
    fi
}
read_dt_str() {
    if [ -r "$1" ]; then
        printf '%s: ' "$1"
        tr '\000' '\n' < "$1" 2>/dev/null || :
        printf '\n'
    fi
}
read_dt_hex() {
    # DT integers are binary big-endian bytes; do NOT treat as ASCII integers.
    if [ -r "$1" ]; then
        printf '%s (binary device-tree property, hex octets):\n' "$1"
        od -tx1 "$1" 2>/dev/null || :
    fi
}

printf '\n=== Linux input devices as identified by the kernel ===\n'
if [ -r /proc/bus/input/devices ]; then
    # Exposes device names/bus and capability bitsets, NOT touch coordinates.
    sed -n '1,180p' /proc/bus/input/devices 2>/dev/null || :
fi

printf '\n=== evdev sysfs: inputs / capabilities / driver links ===\n'
count=0
for base in /sys/class/input/event*/device; do
    [ -d "$base" ] || continue
    count=$((count + 1))
    [ "$count" -le 14 ] || { printf 'EVENT_DEVICE_LIMIT=14\n'; break; }
    printf '\n-- %s --\n' "$base"
    for file in name id/bustype id/vendor id/product id/version \
                capabilities/ev capabilities/key capabilities/rel \
                capabilities/abs capabilities/msc capabilities/sw \
                capabilities/prop; do
        read_property "$base/$file"
    done
    read_link "$base/driver"
    read_link "$base/device/driver"
    read_link "$base/../driver"
done

printf '\n=== Confirm named ILI2117 I2C device binding (no I2C reads) ===\n'
dev=/sys/bus/i2c/devices/4-0026
if [ -d "$dev" ]; then
    for file in name modalias uevent power/runtime_status; do
        read_property "$dev/$file"
    done
    read_link "$dev/driver"
    read_link "$dev/of_node"
    read_link "$dev/subsystem"
    read_dt_str "$dev/of_node/compatible"
    read_dt_str "$dev/of_node/status"
    # Touch properties per Documentation/devicetree/bindings/input/touchscreen.
    # Their *presence* and binary contents can reveal inversion, raw size.
    for prop in touchscreen-size-x touchscreen-size-y \
                touchscreen-fuzz-x touchscreen-fuzz-y \
                touchscreen-inverted-x touchscreen-inverted-y \
                touchscreen-swapped-x-y interrupts irq-gpios reset-gpios; do
        read_dt_hex "$dev/of_node/$prop"
    done
else
    printf 'ILI2117_I2C_DEVICE_4-0026=absent (confirm actual sysfs bus/address)\n'
fi

printf '\n=== I2C and driver inventory (names only) ===\n'
for path in /sys/bus/i2c/drivers/*; do
    [ -d "$path" ] || continue
    case "$path" in
        *ili*|*ILI*|*touch*|*Touch*)
            printf '%s\n' "$path"
            ;;
    esac
done

printf '\n=== DRM/KMS primary DSI display connector ===\n'
base=/sys/class/drm/card0-DSI-1
for item in status enabled modes dpms; do
    read_property "$base/$item"
done
read_link "$base/device"
read_link "$base/driver"

printf '\n=== Framebuffer (metadata only, never /dev/fb0) ===\n'
read_property /proc/fb
for item in name virtual_size bits_per_pixel rotate blank mode modes \
            stride; do
    read_property "/sys/class/graphics/fb0/$item"
done

printf '\n=== GPU DRM render node / binding ===\n'
for item in /sys/class/drm/card1/device/uevent \
            /sys/class/drm/renderD128/device/uevent; do
    read_property "$item"
done
read_link /sys/class/drm/card1/device/driver
read_link /sys/class/drm/renderD128/device/driver

printf '\n=== Backlight sysfs (current values only) ===\n'
for dir in /sys/class/backlight/*; do
    [ -d "$dir" ] || continue
    printf '%s\n' "$dir"
    for item in type brightness actual_brightness max_brightness bl_power; do
        read_property "$dir/$item"
    done
done

printf '\n=== Touch/I2C/display interrupt names and counters ===\n'
if [ -r /proc/interrupts ]; then
    grep -Ei 'ilitek|ili2117|ili210|touch|i2c|vop|dsi|panfrost|mali' \
        /proc/interrupts 2>/dev/null || :
fi

printf '\n=== Factory app touch and DRM handles (fd symlinks ONLY) ===\n'
pid=''
if command -v systemctl >/dev/null 2>&1; then
    pid=$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)
fi
case "$pid" in
    ''|0|*[!0-9]*) printf 'FACTORY_MAINPID=unavailable\n' ;;
    *)
        if [ -d "/proc/$pid/fd" ]; then
            printf 'FACTORY_MAINPID=%s\n' "$pid"
            seen=0
            for fd in /proc/"$pid"/fd/*; do
                [ -L "$fd" ] || continue
                seen=$((seen + 1))
                [ "$seen" -le 256 ] || break
                target=$(readlink "$fd" 2>/dev/null || :)
                case "$target" in
                    /dev/dri/*|/dev/fb*|/dev/input/*|/dev/tty0|/dev/tty1)
                        printf 'FD=%s -> %s\n' "$(basename "$fd")" "$target"
                        ;;
                esac
            done
        else
            printf 'FACTORY_MAINPID=unavailable-after-check\n'
        fi ;;
esac
if command -v systemctl >/dev/null 2>&1; then
    printf '\nFactory service after inspection:\n'
    systemctl show acvs.service --no-pager \
        -p ActiveState -p SubState -p MainPID 2>/dev/null || :
fi

printf 'END MPFREE TOUCHSCREEN AND DISPLAY HARDWARE MAP\n'
printf 'READ-ONLY: no input/DRM/I2C device was opened or modified.\n'
