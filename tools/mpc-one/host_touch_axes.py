#!/usr/bin/env python3
"""M10 exact-original-MPC read-only evdev metadata via independently opted-in /run.

Import mature M5-M7 identity/ABI/RAM preflight, but use a separate allowlisted
remote script and output parser. This module is invoked ONLY by the identity-
gated collect-over-ssh.sh wrapper. No arbitrary shell, file path or command.
"""
import argparse
import hashlib
from pathlib import Path
import re
import shlex
import subprocess
import sys

import host_ram_smoke as safe

BINARY_NAME = "mpfree-touch"
SENTINEL = "MPFREE_TOUCH_ABS_METADATA_V1"
WATCHDOG = "MPFREE_TOUCH_IOCTL_WATCHDOG_5S_ARMED"
AXES = ("abs_x", "abs_y", "abs_mt_slot", "abs_mt_x", "abs_mt_y", "abs_mt_tracking_id")
PROPERTIES = ("min", "max", "fuzz", "flat", "resolution")


def verify_touch_preflight(report):
    lines = report.splitlines()
    for marker in (
        "MPFREE_TOUCH_AXES_PREFLIGHT_V1",
        "touch_driver=ili2117",
        "touch_event=event0",
        "touch_bus=i2c-4-0026",
        "factory_service_active=active",
        "ioctl_performed=no",
        "END MPFREE_TOUCH_AXES_PREFLIGHT",
    ):
        if lines.count(marker) != 1:
            raise RuntimeError(f"missing/repeated touch preflight marker {marker!r}")


def remote_script(digest):
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("invalid SHA-256")
    return r"""set -eu
printf 'STAGE=CHECK_RUN_TMPFS\n'
awk '$2=="/run" && $3=="tmpfs" {found=1} END{exit !found}' /proc/mounts
for tool in mktemp cat sha256sum awk chmod rm rmdir systemctl; do
    command -v "$tool" >/dev/null 2>&1
done
printf 'STAGE=CHECK_STOCK_AND_TOUCH\n'
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null)"
case "$pid" in ''|0|*[!0-9]*) exit 5 ;; esac
test -d "/proc/$pid"
test "$(cat /sys/class/input/event0/device/name)" = "ILI2117 Touchscreen"
test "$(cat /sys/class/input/event0/device/id/bustype)" = "0018"
test "$(cat /sys/bus/i2c/devices/4-0026/name)" = "ili2117"
test -c /dev/input/event0
case "$(readlink /sys/bus/i2c/devices/4-0026/driver)" in
  */ili2117) ;;
  *) exit 5 ;;
esac
printf 'STAGE=CREATE_TMPFS_DIR\n'
umask 077
stage="$(mktemp -d /run/mpfree-touch-axes.XXXXXXXX)"
cleanup() {
    rm -f "$stage/probe" 2>/dev/null || :
    rmdir "$stage" 2>/dev/null || :
}
trap cleanup 0
trap 'exit 124' 1 2 15
printf 'STAGE=RECEIVE_PAYLOAD\n'
cat > "$stage/probe"
chmod 0500 "$stage/probe"
printf 'STAGE=VERIFY_SHA256\n'
observed="$(sha256sum "$stage/probe" | awk '{print $1}')"
test "$observed" = "__HASH__" || { printf 'SHA256_MISMATCH\n' >&2; exit 3; }
printf 'MPFREE_TOUCH_STAGE_SHA256_OK\n'
ulimit -c 0
ulimit -f 0
ulimit -t 3
policy="$(awk '{sub(/^.*\) /, ""); if (NF >= 39) print $39; else print "unavailable"}' /proc/self/stat)"
test "$policy" = "0"
printf 'STAGE=TOUCH_METADATA_IOCTL\n'
# Fixed binary and fixed argument. NO events read, no grab, no calibration.
MPFREE_ACCEPT_TOUCH_IOCTL=YES "$stage/probe" --inspect
printf 'STAGE=RUST_EXECUTED\n'
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
""".replace("__HASH__", digest)


def parse_touch_output(stdout, stderr, code):
    if code != 0:
        raise RuntimeError(
            f"remote touch ioctl exited {code}; "
            f"last stdout={stdout[-600:]!r}; stderr={stderr[-600:]!r}"
        )
    lines = stdout.splitlines()
    required = (
        "MPFREE_TOUCH_STAGE_SHA256_OK",
        "STAGE=RUST_EXECUTED",
        SENTINEL,
    )
    for marker in required:
        if lines.count(marker) != 1:
            raise RuntimeError(f"missing/repeated touch execution marker {marker}")
    if stderr.splitlines().count(WATCHDOG) != 1:
        raise RuntimeError("physical 5s touch worker watchdog not demonstrated")
    fields = {}
    for line in lines:
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        if key == "STAGE":
            continue
        if key in fields:
            raise RuntimeError(f"duplicate touch field {key}")
        fields[key] = value
    fixed = {
        "driver": "ili2117",
        "device": "event0",
        "bus": "i2c-4-0026",
        "access": "evdev_RDONLY",
        "events_consumed": "0",
        "exclusive_grab": "no",
        "status": "PASS",
    }
    for key, value in fixed.items():
        if fields.get(key) != value:
            raise RuntimeError(f"invalid touch result field {key}: {fields.get(key)!r}")

    allowed = set(fixed) | {"multitouch_slots"} | {
        f"{axis}_{prop}" for axis in AXES for prop in PROPERTIES
    }
    unknown = set(fields) - allowed
    if unknown:
        raise RuntimeError(f"unexpected touch result fields: {sorted(unknown)}")
    for axis in AXES:
        lo = None
        hi = None
        for prop in PROPERTIES:
            key = f"{axis}_{prop}"
            value = fields.get(key)
            if value is None or not re.fullmatch(r"-?[0-9]{1,11}", value):
                raise RuntimeError(f"missing/non-integer touch metadata {key}: {value!r}")
            number = int(value)
            if not -(2**31) <= number < 2**31:
                raise RuntimeError(f"touch metadata outside signed i32: {key}")
            if prop in ("fuzz", "flat", "resolution") and number < 0:
                raise RuntimeError(f"negative touch axis noise/resolution {key}")
            if prop == "min":
                lo = number
            if prop == "max":
                hi = number
        if lo is None or hi is None or lo > hi:
            raise RuntimeError(f"invalid touch axis range {axis}")
        if axis in ("abs_x", "abs_y", "abs_mt_x", "abs_mt_y") and lo == hi:
            raise RuntimeError(f"zero-length touch coordinate range {axis}")
    low, high = int(fields["abs_mt_slot_min"]), int(fields["abs_mt_slot_max"])
    if low != 0 or not 0 <= high <= 63:
        raise RuntimeError("invalid multitouch slot index range")
    slots = int(fields.get("multitouch_slots", "-1"))
    if slots != high + 1:
        raise RuntimeError("incorrect inclusive multitouch slot count")
    return fields


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("ip", "user", "key", "binary", "abi-script", "abi-checker",
                "env-script", "touch-script"):
        parser.add_argument("--" + key, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--execute", action="store_true")
    a = parser.parse_args(argv)
    if not 1 <= a.port <= 65535:
        raise ValueError("invalid port")
    path = Path(a.binary)
    if path.name != BINARY_NAME or not path.is_file():
        raise ValueError("only existing mpfree-touch ARMv7 binary accepted")
    if not 0 < path.stat().st_size <= 2 * 1024 * 1024:
        raise ValueError("touch binary exceeds 2MiB RAM budget")
    image = path.read_bytes()
    for required in (SENTINEL, WATCHDOG):
        if required.encode("ascii") not in image:
            raise ValueError(f"stale touch binary missing {required}")
    digest = hashlib.sha256(image).hexdigest()

    print("M10 live original MPC identity and ARMv7 ABI preflight; NO device writes.", flush=True)
    code, out, err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s", Path(a.abi_script).read_bytes(), 35
    )
    if code:
        raise RuntimeError(f"ABI SSH survey failed: {err[-500:]}")
    safe.verify_live_board(out)
    code, env_out, env_err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s", Path(a.env_script).read_bytes(), 25
    )
    if code:
        raise RuntimeError(f"runtime prerequisite SSH failed: {env_err[-500:]}")
    print(safe.check_remote_environment(env_out), flush=True)
    code, touch_out, touch_err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s",
        Path(a.touch_script).read_bytes(), 20
    )
    if code:
        raise RuntimeError(f"touch sysfs preflight failed: {touch_err[-600:]}")
    verify_touch_preflight(touch_out)
    print("PASS: ILI2117 input event0 and I2C binding confirmed, factory active.", flush=True)
    safe.preflight_elf(Path(a.abi_checker), out, path, "arm-linux-gnueabihf-readelf")
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise RuntimeError("touch executable changed during ABI preflight")
    print(f"Candidate bytes={len(image)}, SHA256={digest}", flush=True)
    if not a.execute:
        print("PREFLIGHT ONLY: no binary uploaded and no /dev/input node opened.")
        return 0

    print("OWNER DUAL OPT-IN: volatile /run touch metadata-only EVIOCGABS.", flush=True)
    command = "sh -c " + shlex.quote(remote_script(digest))
    code, stdout, stderr = safe.call_ssh(
        a.ip, a.port, a.user, a.key, command, image, 18
    )
    fields = parse_touch_output(stdout, stderr, code)
    print(stdout.strip())
    print(WATCHDOG)
    print(f"multitouch_slots={fields['multitouch_slots']}")
    print("PASS: bounded touch axis metadata inspection; stock app active at exit.")
    print("CAUTION: did not sample gestures, determine orientation or perform app handoff.")
    print("Remote cleanup trap removes volatile /run stage after shell completion.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"M10 FAIL CLOSED: {exc}", file=sys.stderr)
        sys.exit(2)
