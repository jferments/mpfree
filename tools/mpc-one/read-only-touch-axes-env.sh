#!/bin/sh
# Original MPC One, exact ILI2117 touch metadata preflight.
# NO /dev/input open, no ioctl, no I2C transactions, no device-state changes.
set -eu
printf 'MPFREE_TOUCH_AXES_PREFLIGHT_V1\n'
[ -r /sys/class/input/event0/device/name ]
[ -r /sys/class/input/event0/device/id/bustype ]
[ -r /sys/bus/i2c/devices/4-0026/name ]
[ -r /sys/bus/i2c/devices/4-0026/of_node/compatible ]
[ -L /sys/bus/i2c/devices/4-0026/driver ]
[ -c /dev/input/event0 ]
[ "$(cat /sys/class/input/event0/device/name)" = "ILI2117 Touchscreen" ]
[ "$(cat /sys/class/input/event0/device/id/bustype)" = "0018" ]
[ "$(cat /sys/bus/i2c/devices/4-0026/name)" = "ili2117" ]
case "$(readlink /sys/bus/i2c/devices/4-0026/driver)" in
  */ili2117) ;;
  *) printf 'touch driver unexpected\n' >&2; exit 3 ;;
esac
tr '\000' '\n' < /sys/bus/i2c/devices/4-0026/of_node/compatible | grep -Fxq 'ilitek,ili2117'
# Event capabilities are read from sysfs; all real ioctl axis measurements
# are deferred to separately authorized owner-initiated M10 execution.
[ -r /sys/class/input/event0/device/capabilities/abs ]
[ -r /sys/class/input/event0/device/capabilities/ev ]
[ "$(systemctl is-active acvs.service 2>/dev/null)" = "active" ]
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null)"
case "$pid" in ''|0|*[!0-9]*) exit 4 ;; esac
[ -d "/proc/$pid" ]
printf 'touch_driver=ili2117\n'
printf 'touch_event=event0\n'
printf 'touch_bus=i2c-4-0026\n'
printf 'factory_service_active=active\n'
printf 'ioctl_performed=no\n'
printf 'END MPFREE_TOUCH_AXES_PREFLIGHT\n'
