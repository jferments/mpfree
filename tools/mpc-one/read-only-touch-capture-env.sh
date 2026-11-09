#!/bin/sh
# M12 read-only preflight for the ILI2117 touch capture.
# Reads sysfs, procfs and systemd state only. Opens no /dev node, runs no
# ioctl, writes nothing. Also records the eMMC boot-chain write counters so
# the capture run can prove it caused zero writes to p1-p6.
set -eu
printf 'MPFREE_TOUCH_CAPTURE_PREFLIGHT_V1\n'
[ "$(cat /sys/class/input/event0/device/name)" = "ILI2117 Touchscreen" ]
[ "$(cat /sys/class/input/event0/device/id/bustype)" = "0018" ]
[ "$(cat /sys/bus/i2c/devices/4-0026/name)" = "ili2117" ]
case "$(readlink /sys/bus/i2c/devices/4-0026/driver)" in
  */ili2117) ;;
  *) printf 'touch driver unexpected\n' >&2; exit 3 ;;
esac
tr '\000' '\n' < /sys/bus/i2c/devices/4-0026/of_node/compatible | grep -Fxq 'ilitek,ili2117'
[ -c /dev/input/event0 ]
[ "$(systemctl is-active acvs.service 2>/dev/null)" = "active" ]
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null)"
case "$pid" in ''|0|*[!0-9]*) exit 4 ;; esac
[ -d "/proc/$pid" ]
# The eMMC must be mmcblk0 with the partition names recorded in
# docs/hardware/MPC_ONE_3_6_0_BASELINE.md before its counters mean anything.
n=1
for name in uboot-spl env uboot splash recoverysplash rootfs; do
  grep -Fxq "PARTNAME=$name" "/sys/class/block/mmcblk0p$n/uevent" || {
    printf 'mmcblk0p%s is not %s\n' "$n" "$name" >&2; exit 5; }
  # stat field 5 = writes completed, field 7 = sectors written.
  awk -v n="$n" '{print "emmc_p" n "_writes=" $5 " emmc_p" n "_sectors_written=" $7}' \
    "/sys/class/block/mmcblk0p$n/stat"
  n=$((n + 1))
done
printf 'touch_driver=ili2117\n'
printf 'touch_event=event0\n'
printf 'factory_service_active=active\n'
printf 'device_node_opened=no\n'
printf 'END MPFREE_TOUCH_CAPTURE_PREFLIGHT\n'
