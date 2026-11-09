#!/usr/bin/env python3
"""M12: guarded 60-second read-only ILI2117 touch capture.

Invoked ONLY by the identity-gated collect-over-ssh.sh wrapper. It reuses the
M5-M10 identity, ABI and RAM preflights, then (only with --execute) stages the
fixed, hashed mpfree-touch-capture binary in /run, runs it once, streams its
event log back, prints timed touch prompts, and checks afterwards that the
factory app is still active and that eMMC partitions p1-p6 saw zero writes.

The capture binary opens /dev/input/event0 read-only and non-blocking. It
never grabs the device, never issues an ioctl, and writes nothing on the MPC.
"""
import argparse
import hashlib
from pathlib import Path
import re
import shlex
import subprocess
import sys
import threading
import time

import host_ram_smoke as safe

BINARY_NAME = "mpfree-touch-capture"
HEADER = "MPFREE_TOUCH_CAPTURE_V1"
ARMED = "MPFREE_TOUCH_CAPTURE_ARMED"
END = "MPFREE_TOUCH_CAPTURE_END"
WATCHDOG = "MPFREE_TOUCH_CAPTURE_WATCHDOG_70S_ARMED"
PARTITIONS = range(1, 7)
# Seconds after the device reports ARMED, and what to do next.
PROMPTS = (
    (0, "Get ready. First: one finger in the CENTRE."),
    (3, "NOW: hold one finger still in the CENTRE for 3 s, then lift."),
    (8, "TOP-LEFT corner: hold about 1 s, then lift."),
    (12, "TOP-RIGHT corner: hold about 1 s, then lift."),
    (16, "BOTTOM-RIGHT corner: hold about 1 s, then lift."),
    (20, "BOTTOM-LEFT corner: hold about 1 s, then lift."),
    (24, "Drag one finger slowly LEFT to RIGHT across the middle, then lift."),
    (28, "Drag one finger slowly TOP to BOTTOM through the middle, then lift."),
    (32, "TWO fingers down together, hold 2 s, then lift."),
    (37, "FIVE fingers (one flat hand), hold 2 s, then lift."),
    (43, "Optional: TEN fingers (both hands), hold 2 s, then lift."),
    (50, "Done. Keep hands off the screen; capture stops at 60 s."),
)
EXECUTE_DEADLINE_S = 110


def parse_preflight(report):
    """Return the eMMC counters after checking every marker exactly once."""
    lines = report.splitlines()
    for marker in (
        "MPFREE_TOUCH_CAPTURE_PREFLIGHT_V1",
        "touch_driver=ili2117",
        "touch_event=event0",
        "factory_service_active=active",
        "device_node_opened=no",
        "END MPFREE_TOUCH_CAPTURE_PREFLIGHT",
    ):
        if lines.count(marker) != 1:
            raise RuntimeError(f"missing/repeated touch capture preflight marker {marker!r}")
    return parse_counters(report, "")


def parse_counters(text, prefix):
    counters = {}
    for token in text.split():
        m = re.fullmatch(prefix + r"emmc_p([1-6])_(writes|sectors_written)=([0-9]{1,20})", token)
        if m:
            key = (int(m.group(1)), m.group(2))
            if key in counters:
                raise RuntimeError(f"duplicate eMMC counter {prefix}{key}")
            counters[key] = int(m.group(3))
    want = {(p, k) for p in PARTITIONS for k in ("writes", "sectors_written")}
    if set(counters) != want:
        raise RuntimeError(f"incomplete {prefix or 'preflight '}eMMC counters")
    return counters


def counters_script(prefix):
    return (
        "n=1\n"
        "for name in uboot-spl env uboot splash recoverysplash rootfs; do\n"
        '  grep -Fxq "PARTNAME=$name" "/sys/class/block/mmcblk0p$n/uevent"\n'
        f"  awk -v n=\"$n\" '{{print \"{prefix}emmc_p\" n \"_writes=\" $5 "
        f"\" {prefix}emmc_p\" n \"_sectors_written=\" $7}}' "
        '"/sys/class/block/mmcblk0p$n/stat"\n'
        "  n=$((n + 1))\n"
        "done\n"
    )


def remote_script(digest):
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("invalid SHA-256")
    return r"""set -eu
printf 'STAGE=CHECK_RUN_TMPFS\n'
awk '$2=="/run" && $3=="tmpfs" {found=1} END{exit !found}' /proc/mounts
for tool in mktemp cat sha256sum awk chmod rm rmdir systemctl grep readlink; do
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
printf 'STAGE=EMMC_COUNTERS_BEFORE\n'
__PRE__
printf 'STAGE=CREATE_TMPFS_DIR\n'
umask 077
stage="$(mktemp -d /run/mpfree-touch-capture.XXXXXXXX)"
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
printf 'MPFREE_TOUCH_CAPTURE_STAGE_SHA256_OK\n'
ulimit -c 0
ulimit -f 0
ulimit -t 15
policy="$(awk '{sub(/^.*\) /, ""); if (NF >= 39) print $39; else print "unavailable"}' /proc/self/stat)"
test "$policy" = "0"
printf 'STAGE=TOUCH_CAPTURE\n'
# Fixed binary, fixed argument. Read-only event0, no grab, no ioctl.
status=0
MPFREE_ACCEPT_TOUCH_CAPTURE=YES "$stage/probe" --capture || status=$?
printf 'STAGE=CAPTURE_EXITED status=%s\n' "$status"
cleanup
test ! -e "$stage" && printf 'STAGE_DIR_REMOVED\n'
printf 'STAGE=EMMC_COUNTERS_AFTER\n'
__POST__
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
printf 'FACTORY_ACTIVE_AFTER\n'
exit "$status"
""".replace("__HASH__", digest).replace(
        "__PRE__", counters_script("pre_")).replace(
        "__POST__", counters_script("post_"))


def check_execution(stdout, stderr, code):
    """Validate the run. Returns the capture footer fields."""
    lines = stdout.splitlines()
    if code != 0:
        raise RuntimeError(
            f"remote capture exited {code}; stdout tail={stdout[-600:]!r}; "
            f"stderr={stderr[-600:]!r}"
        )
    for marker in (
        "MPFREE_TOUCH_CAPTURE_STAGE_SHA256_OK", HEADER, ARMED,
        "STAGE=CAPTURE_EXITED status=0", "STAGE_DIR_REMOVED", "FACTORY_ACTIVE_AFTER",
    ):
        if lines.count(marker) != 1:
            raise RuntimeError(f"missing/repeated capture marker {marker}")
    if stderr.splitlines().count(WATCHDOG) != 1:
        raise RuntimeError("capture watchdog not demonstrated")
    before = parse_counters(stdout, "pre_")
    after = parse_counters(stdout, "post_")
    if before != after:
        changed = sorted(k for k in before if before[k] != after[k])
        raise RuntimeError(f"eMMC boot-chain counters changed during capture: {changed}")
    footer = [l for l in lines if l.startswith(END + " ")]
    if len(footer) != 1:
        raise RuntimeError("missing/repeated capture footer")
    fields = dict(kv.split("=", 1) for kv in footer[0].split()[1:] if "=" in kv)
    if fields.get("status") != "PASS":
        raise RuntimeError(f"capture status {fields.get('status')!r}")
    events = sum(1 for l in lines if l.startswith("E "))
    if str(events) != fields.get("events"):
        raise RuntimeError("streamed event count does not match capture footer")
    if events == 0:
        raise RuntimeError(
            "zero touch events received: the factory app may hold an exclusive "
            "grab on event0, or the screen was not touched"
        )
    return fields


def stream_capture(command, image, deadline_s, log_path, prompts, out):
    """Run the remote capture, stream stdout to the log, print prompts."""
    proc = subprocess.Popen(
        command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    armed = threading.Event()
    stderr_chunks = []

    def drain_stderr():
        stderr_chunks.append(proc.stderr.read())

    def prompt():
        if not armed.wait(deadline_s):
            return
        start = time.monotonic()
        for at, text in prompts:
            delay = start + at - time.monotonic()
            if delay > 0 and proc.poll() is None:
                time.sleep(delay)
            if proc.poll() is not None:
                return
            out(f"[{at:2d}s] {text}")

    killer = threading.Timer(deadline_s, proc.kill)
    threads = [threading.Thread(target=drain_stderr, daemon=True),
               threading.Thread(target=prompt, daemon=True)]
    for t in threads:
        t.start()
    killer.start()
    stdout_lines = []
    try:
        proc.stdin.write(image)
        proc.stdin.close()
        with open(log_path, "x", encoding="utf-8") as log:
            for raw in proc.stdout:
                line = raw.decode("utf-8", "replace").rstrip("\n")
                stdout_lines.append(line)
                log.write(line + "\n")
                if line == ARMED:
                    armed.set()
                    out("Capture armed: 60 s window starts now.")
        code = proc.wait()
    finally:
        killer.cancel()
        armed.set()
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    threads[0].join(5)
    proc.stdout.close()
    proc.stderr.close()
    stderr = b"".join(c for c in stderr_chunks if c).decode("utf-8", "replace")
    return code, "\n".join(stdout_lines) + "\n", stderr


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("ip", "user", "key", "binary", "abi-script", "abi-checker",
                "env-script", "capture-script", "report"):
        parser.add_argument("--" + key, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--execute", action="store_true")
    a = parser.parse_args(argv)
    report = open(a.report, "a", encoding="utf-8")

    def out(text):
        print(text, flush=True)
        report.write(text + "\n")
        report.flush()

    if not 1 <= a.port <= 65535:
        raise ValueError("invalid port")
    path = Path(a.binary)
    if path.name != BINARY_NAME or not path.is_file():
        raise ValueError("only the existing mpfree-touch-capture ARMv7 binary is accepted")
    if not 0 < path.stat().st_size <= 2 * 1024 * 1024:
        raise ValueError("capture binary exceeds 2 MiB RAM budget")
    image = path.read_bytes()
    for required in (HEADER, ARMED, WATCHDOG):
        if required.encode("ascii") not in image:
            raise ValueError(f"stale capture binary missing {required}")
    digest = hashlib.sha256(image).hexdigest()

    out("M12 live MPC identity and ARMv7 ABI preflight; read-only.")
    code, abi, err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s", Path(a.abi_script).read_bytes(), 35)
    if code:
        raise RuntimeError(f"ABI SSH survey failed: {err[-500:]}")
    safe.verify_live_board(abi)
    code, env_out, env_err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s", Path(a.env_script).read_bytes(), 25)
    if code:
        raise RuntimeError(f"runtime prerequisite SSH failed: {env_err[-500:]}")
    out(safe.check_remote_environment(env_out))
    code, pre_out, pre_err = safe.call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s", Path(a.capture_script).read_bytes(), 20)
    if code:
        raise RuntimeError(f"touch capture preflight failed: {pre_err[-600:]}")
    counters = parse_preflight(pre_out)
    out("PASS: ILI2117 event0 and I2C binding confirmed, factory app active.")
    out("eMMC p1-p6 writes completed: " + " ".join(
        f"p{p}={counters[(p, 'writes')]}" for p in PARTITIONS))
    safe.preflight_elf(Path(a.abi_checker), abi, path, "arm-linux-gnueabihf-readelf")
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise RuntimeError("capture binary changed during ABI preflight")
    out(f"Candidate {BINARY_NAME} bytes={len(image)} SHA256={digest}")
    if not a.execute:
        out("PREFLIGHT ONLY: nothing uploaded, no /dev/input node opened.")
        return 0

    log_path = Path(a.report).with_suffix(".events.log")
    out("OWNER DUAL OPT-IN: 60 s read-only event0 capture from /run.")
    out("Follow the prompts below. Touches also reach the Akai app as usual.")
    command = safe.ssh_command(
        a.ip, a.port, a.user, a.key, "sh -c " + shlex.quote(remote_script(digest)))
    code, stdout, stderr = stream_capture(
        command, image, EXECUTE_DEADLINE_S, log_path, PROMPTS, out)
    fields = check_execution(stdout, stderr, code)
    out(f"capture events={fields.get('events')} drops={fields.get('drops')} "
        f"elapsed_ms={fields.get('elapsed_ms')}")
    out("PASS: eMMC p1-p6 counters unchanged, /run stage removed, factory app active.")
    out(f"Event log: {log_path}")
    out("Next: run mpfree-touch-analyze on the event log (see runbook).")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"M12 FAIL CLOSED: {exc}", file=sys.stderr)
        sys.exit(2)
