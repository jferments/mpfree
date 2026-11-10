#!/usr/bin/env python3
"""M12: guarded, owner-paced, read-only ILI2117 touch capture.

Invoked ONLY by the identity-gated collect-over-ssh.sh wrapper. It reuses the
M5-M10 identity, ABI and RAM preflights. Then, only with --execute, it walks
the owner through the touch steps one at a time: for each step it shows the
instruction, waits for Enter, stages the fixed, hashed mpfree-touch-capture
binary in /run, runs it for one 9-second window (3-second countdown, then 6
seconds to do the step), and checks afterwards that the stage directory is
gone, the factory app is still active and eMMC p1-p6 saw zero writes.

The capture binary opens /dev/input/event0 read-only and non-blocking. It
never grabs the device, never issues an ioctl, and writes nothing on the MPC.
Ctrl-C at any point closes the SSH session; the MPC side then stops within
the 9-second window and its EXIT trap deletes the /run copy.
"""
import argparse
import hashlib
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import threading
import time

import host_ram_smoke as safe

BINARY_NAME = "mpfree-touch-capture"
HEADER = "MPFREE_TOUCH_CAPTURE_V2"
ARMED = "MPFREE_TOUCH_CAPTURE_ARMED"
END = "MPFREE_TOUCH_CAPTURE_END"
WATCHDOG = "MPFREE_TOUCH_CAPTURE_WATCHDOG_20S_ARMED"
LOG_HEADER = "MPFREE_TOUCH_CAPTURE_STEPS_V1"
LOG_END = "MPFREE_TOUCH_CAPTURE_STEPS_END"
PARTITIONS = range(1, 7)
# Must equal CAPTURE_SECONDS in rust/mpfree-touch/src/bin/capture.rs.
STEP_SECONDS = 9
COUNTDOWN_SECONDS = 3
# Per step: SSH connect, upload, the 9 s capture and the after-checks.
STEP_DEADLINE_S = 45
ABS_MT_TRACKING_ID = 57
# (name, fingers, optional, instruction). Names must match STEP_NAMES in
# rust/mpfree-touch/src/calibrate.rs. "Top-left" is as you face the MPC.
STEPS = (
    ("centre", 1, False,
     "Hold ONE finger still in the CENTRE of the screen for 3 seconds, then lift."),
    ("top_left", 1, False,
     "Touch the TOP-LEFT corner, as close to the edges as you can. Hold 1 second, then lift."),
    ("top_right", 1, False,
     "Touch the TOP-RIGHT corner, as close to the edges as you can. Hold 1 second, then lift."),
    ("bottom_right", 1, False,
     "Touch the BOTTOM-RIGHT corner, as close to the edges as you can. Hold 1 second, then lift."),
    ("bottom_left", 1, False,
     "Touch the BOTTOM-LEFT corner, as close to the edges as you can. Hold 1 second, then lift."),
    ("drag_left_to_right", 1, False,
     "Drag ONE finger slowly from the LEFT edge to the RIGHT edge across the middle, then lift."),
    ("drag_top_to_bottom", 1, False,
     "Drag ONE finger slowly from the TOP edge to the BOTTOM edge through the middle, then lift."),
    ("two_fingers", 2, False,
     "Put TWO fingers down together, hold 2 seconds, then lift."),
    ("five_fingers", 5, False,
     "Put FIVE fingers down (one flat hand), hold 2 seconds, then lift."),
    ("ten_fingers", 10, True,
     "Optional: put TEN fingers down (both hands), hold 2 seconds, then lift."),
)


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
# If the SSH link closes early (Ctrl-C on the workstation), writes fail with
# EPIPE instead of killing the shell, so the EXIT trap below always runs.
trap '' PIPE
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
# Fixed binary, fixed argument, fixed 9 s. Read-only event0, no grab, no ioctl.
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


def check_execution(stdout, stderr, code, baseline):
    """Validate one step's run. Returns (footer fields, capture lines).

    `baseline` is the eMMC p1-p6 counters from the read-only preflight. Both
    the before and after counters of every step must equal it exactly.
    """
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
    for label, seen in (("before", before), ("after", after)):
        if seen != baseline:
            changed = sorted(k for k in baseline if baseline[k] != seen[k])
            raise RuntimeError(
                f"eMMC boot-chain counters {label} this step differ from the preflight: {changed}")
    footer = [l for l in lines if l.startswith(END + " ")]
    if len(footer) != 1:
        raise RuntimeError("missing/repeated capture footer")
    fields = dict(kv.split("=", 1) for kv in footer[0].split()[1:] if "=" in kv)
    if fields.get("status") != "PASS":
        raise RuntimeError(f"capture status {fields.get('status')!r}")
    first, last = lines.index(HEADER), lines.index(footer[0])
    if not first < lines.index(ARMED) < last:
        raise RuntimeError("capture markers out of order")
    capture = lines[first:last + 1]
    events = sum(1 for l in capture if l.startswith("E "))
    if str(events) != fields.get("events"):
        raise RuntimeError("streamed event count does not match capture footer")
    return fields, capture


def count_touches(capture):
    """New contacts in a step: ABS_MT_TRACKING_ID events with a value >= 0."""
    n = 0
    for line in capture:
        f = line.split()
        if (len(f) == 6 and f[0] == "E" and f[3] == "3"
                and f[4] == str(ABS_MT_TRACKING_ID) and not f[5].startswith("-")):
            n += 1
    return n


def run_step(command, image, deadline_s, go_text, out, countdown=COUNTDOWN_SECONDS):
    """Run one remote capture: upload, countdown once armed, GO, then STOP.

    Returns (exit code, stdout, stderr). Kills the local ssh client on the
    deadline or on any exception (including Ctrl-C).
    """
    proc = subprocess.Popen(
        command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    lines, err_chunks = [], []
    armed, ended, eof = threading.Event(), threading.Event(), threading.Event()

    def read_stdout():
        try:
            for raw in proc.stdout:
                line = raw.decode("utf-8", "replace").rstrip("\n")
                lines.append(line)
                if line == ARMED:
                    armed.set()
                elif line.startswith(END + " "):
                    ended.set()
        finally:
            eof.set()

    def read_stderr():
        err_chunks.append(proc.stderr.read())

    readers = [threading.Thread(target=read_stdout, daemon=True),
               threading.Thread(target=read_stderr, daemon=True)]
    for t in readers:
        t.start()
    stop_at = time.monotonic() + deadline_s

    def wait_for(event):
        while not event.is_set() and not eof.is_set():
            left = stop_at - time.monotonic()
            if left <= 0:
                return False
            event.wait(min(left, 0.1))
        return event.is_set()

    try:
        try:
            proc.stdin.write(image)
            proc.stdin.close()
        except BrokenPipeError:
            pass  # remote side ended early; its output says why
        if wait_for(armed):
            for n in range(countdown, 0, -1):
                if ended.is_set() or eof.is_set():
                    break
                out(f"  {n}...")
                time.sleep(1)
            if not (ended.is_set() or eof.is_set()):
                out(f"GO: {go_text}  ({STEP_SECONDS - countdown} s)")
            if wait_for(ended):
                out("STOP. Lift your fingers.")
        left = stop_at - time.monotonic()
        code = proc.wait(timeout=max(left, 0.1))
    except subprocess.TimeoutExpired:
        code = None
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    for t in readers:
        t.join(5)
    proc.stdout.close()
    proc.stderr.close()
    if code is None:
        code = proc.returncode
        out(f"step exceeded {deadline_s} s; ssh closed")
    stderr = b"".join(c for c in err_chunks if c).decode("utf-8", "replace")
    return code, "\n".join(lines) + "\n", stderr


def run_steps(command, image, baseline, log_path, out, ask, steps=STEPS,
              deadline_s=STEP_DEADLINE_S, countdown=COUNTDOWN_SECONDS):
    """Walk the owner through every step. Each step is its own guarded run.

    `ask(prompt)` reads one line from the owner (input() in real use).
    Returns the number of steps kept in the event log.
    """
    kept = 0
    total_events = 0
    with open(log_path, "x", encoding="utf-8") as log:
        log.write(LOG_HEADER + "\n")
        log.flush()
        for i, (name, fingers, optional, text) in enumerate(steps, 1):
            while True:
                out("")
                out(f"[{i}/{len(steps)}] {text}")
                prompt = "Press Enter when you are ready"
                if optional:
                    prompt += " (or type s then Enter to skip)"
                reply = ask(prompt + ": ").strip().lower()
                if optional and reply == "s":
                    out(f"step {name}: skipped")
                    break
                out("Starting the capture on the MPC...")
                code, stdout, stderr = run_step(
                    command, image, deadline_s, text, out, countdown)
                fields, capture = check_execution(stdout, stderr, code, baseline)
                touches = count_touches(capture)
                out(f"step {name}: events={fields.get('events')} touches={touches} "
                    f"drops={fields.get('drops')}; eMMC p1-p6 unchanged, /run copy "
                    "removed, factory app active")
                if fields.get("events") == "0":
                    out("No events reached the capture. If this repeats, the factory app "
                        "may hold an exclusive grab on event0; press Ctrl-C to stop.")
                if touches != fingers:
                    out(f"Expected {fingers} touch(es), saw {touches}.")
                    again = ask("Press Enter to redo this step, or type k then Enter "
                                "to keep it anyway: ").strip().lower()
                    if again != "k":
                        continue
                log.write(f"STEP {name}\n")
                log.write("\n".join(capture) + "\n")
                log.flush()
                kept += 1
                total_events += int(fields.get("events", "0"))
                break
        if total_events == 0:
            raise RuntimeError(
                "zero touch events in every step: the factory app may hold an "
                "exclusive grab on event0, or the screen was not touched")
        log.write(f"{LOG_END} steps={kept}\n")
    return kept


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
            raise ValueError(f"stale capture binary missing {required}; rebuild it")
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
    baseline = parse_preflight(pre_out)
    out("PASS: ILI2117 event0 and I2C binding confirmed, factory app active.")
    out("eMMC p1-p6 writes completed: " + " ".join(
        f"p{p}={baseline[(p, 'writes')]}" for p in PARTITIONS))
    stale = stale_stage_dirs(pre_out)
    if stale:
        out(f"NOTE: {stale} leftover /run/mpfree-touch-capture.* folder(s) from an "
            "interrupted run. They are in RAM only and a reboot removes them.")
    else:
        out("No leftover /run/mpfree-touch-capture.* folders.")
    safe.preflight_elf(Path(a.abi_checker), abi, path, "arm-linux-gnueabihf-readelf")
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise RuntimeError("capture binary changed during ABI preflight")
    out(f"Candidate {BINARY_NAME} bytes={len(image)} SHA256={digest}")
    if not a.execute:
        out("PREFLIGHT ONLY: nothing uploaded, no /dev/input node opened.")
        return 0

    log_path = Path(a.report).with_suffix(".events.log")
    out(f"OWNER DUAL OPT-IN: {len(STEPS)} read-only event0 capture steps from /run.")
    out("For each step: read it, press Enter, wait for the 3-2-1 countdown, then do it")
    out(f"during the {STEP_SECONDS - COUNTDOWN_SECONDS} s window. Touches also reach the "
        "Akai app as usual. Ctrl-C stops at any time.")
    command = safe.ssh_command(
        a.ip, a.port, a.user, a.key, "sh -c " + shlex.quote(remote_script(digest)))
    kept = run_steps(command, image, baseline, log_path, out, input)
    out("")
    out(f"PASS: {kept} steps captured; every step left eMMC p1-p6 unchanged, removed "
        "its /run copy, and the factory app stayed active.")
    out(f"Event log: {log_path}")
    out("Next: run mpfree-touch-analyze on the event log (see runbook).")
    return 0


def stale_stage_dirs(preflight):
    for line in preflight.splitlines():
        m = re.fullmatch(r"stale_stage_dirs=([0-9]{1,6})", line)
        if m:
            return int(m.group(1))
    raise RuntimeError("touch capture preflight did not report stale_stage_dirs")


_interrupted = False


def _on_signal(signum, frame):
    """First Ctrl-C (or TERM from the wrapper's timeout) stops the run; any
    repeat is ignored so it cannot cut the cleanup short."""
    global _interrupted
    if _interrupted:
        return
    _interrupted = True
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGINT, _on_signal)
    signal.signal(signal.SIGTERM, _on_signal)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\nM12 STOPPED BY OWNER: ssh closed. On the MPC the capture ends within "
              f"{STEP_SECONDS} s and its /run copy is deleted. Nothing else was changed.",
              file=sys.stderr)
        sys.exit(130)
    except EOFError:
        print("M12 STOPPED: no terminal input (run it from an interactive terminal).",
              file=sys.stderr)
        sys.exit(2)
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"M12 FAIL CLOSED: {exc}", file=sys.stderr)
        sys.exit(2)
