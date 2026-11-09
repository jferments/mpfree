#!/usr/bin/env python3
"""Host-only orchestrator for guarded volatile ARMv7 Rust MPC userspace smoke.

Only invoke through collect-over-ssh.sh, whose identity guard first checks
the exact Ethernet interface, route, neighbor IP/MAC and strict SSH host key.
Preflight reads the device ABI. Optional run uploads only an expected, hashed,
2MiB-max Rust binary to /run tmpfs, launches under remote timeout, cleans up.
Never touches systemd, ALSA, DRM, MIDI, input nodes or persistent partitions.
"""
import argparse
import hashlib
import importlib.util
from pathlib import Path
import re
import shlex
import subprocess
import sys

BOARD_MODEL = "Akai Professional MPC One"
EXPECTED_KERNEL = "6.6.95-az01-"
SMOKE_SENTINEL = "MPFREE_ARMV7_SMOKE_V1"


def ssh_command(ip, port, user, key, remote_command):
    return [
        "ssh", "-T", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
        "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
        "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes",
        "-i", key, "-p", str(port), f"{user}@{ip}", remote_command,
    ]


def call_ssh(ip, port, user, key, command, data, seconds):
    p = subprocess.run(
        ssh_command(ip, port, user, key, command), input=data,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=seconds,
        check=False,
    )
    return (p.returncode, p.stdout.decode("utf-8", "replace"),
            p.stderr.decode("utf-8", "replace"))


def verify_live_board(report):
    required = (
        "MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY",
        f"board_model={BOARD_MODEL}",
        "ID=az01",
        'VERSION="5.0.8 (scarthgap)"',
        EXPECTED_KERNEL,
        "GNU libc release banner: GNU C Library (GNU libc) "
        "stable release version 2.39.",
    )
    lines = report.splitlines()
    for marker in required:
        # Model, firmware and libc identity must be exact whole lines; a
        # substring match would dangerously accept "MPC One+" as "MPC One".
        found = (
            any(EXPECTED_KERNEL in line for line in lines)
            if marker == EXPECTED_KERNEL else marker in lines
        )
        if not found:
            raise RuntimeError(f"device identity/ABI marker not established: {marker}")


def preflight_elf(checker_path, report, binary, readelf):
    spec = importlib.util.spec_from_file_location("mpfree_checked_arm_abi", checker_path)
    if spec is None or spec.loader is None:
        raise RuntimeError("ABI checker not importable")
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    code, messages = checker.assess(
        report,
        checker.readelf(binary, readelf, ["-h"]),
        checker.readelf(binary, readelf, ["-l"]),
        checker.readelf(binary, readelf, ["-W", "--version-info"]),
        checker.readelf(binary, readelf, ["-d"]),
    )
    for message in messages:
        print(message, flush=True)
    if code != 0:
        raise RuntimeError(f"ELF ABI not qualified (ABI checker exit {code})")



REQUIRED_RUNTIME_TOOLS = (
    "awk", "mktemp", "cat", "sha256sum",
    "chmod", "rm", "rmdir", "systemctl",
)
# Informational-only: the owner's az0x image does NOT contain these commands.
OPTIONAL_RUNTIME_TOOLS = ("timeout", "nice")
RUST_WATCHDOG_MARKER = "WATCHDOG=RUST_5S_ARMED"




# Strictly fixed executables and outputs. The user cannot choose an arbitrary
# target binary, device endpoint, program arguments, or remote shell command.
# "device-smoke" preserves M5 behavior; "headless" is a separate M6 gate.
PROFILES = {
    "device-smoke": {
        "binary": "mpfree-device-smoke",
        "sentinel": "MPFREE_ARMV7_SMOKE_V1",
        "watchdog": RUST_WATCHDOG_MARKER,
        "expected": {
            "os": "linux", "arch": "arm", "sample_rate_hz": "44100",
            "frames": "4096", "beats": "1", "status": "PASS",
        },
    },
    "session": {
        # M7 is a *finite* 2-second NULL-audio timing test. It never opens
        # factory ALSA/DRM/evdev devices and never stops stock services.
        "binary": "mpfree-session",
        "sentinel": "MPFREE_SESSION_V1",
        "watchdog": "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED",
        "expected": {
            "os": "linux", "arch": "arm", "backend": "null",
            "pacing": "monotonic", "sample_rate_hz": "44100",
            "period_frames": "128", "frames": "88200",
            "callback_count": "690", "groove_events": "12",
            "sampler_notes": "4",
            "checksum_fnv1a64": "3dc4d6dabbb17fbd", "status": "PASS",
        },
    },
    "headless": {
        "binary": "mpfree-headless",
        "sentinel": "MPFREE_HEADLESS_V1",
        "watchdog": "MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED",
        "expected": {
            "os": "linux", "arch": "arm", "backend": "null",
            "sample_rate_hz": "44100", "frames": "88200",
            "groove_events": "12", "sampler_notes": "4",
            "groove_voice_steals": "0", "sampler_voice_steals": "0",
            "prepared_bytes": "94628", "status": "PASS",
        },
    },
}


def get_profile(name):
    if name not in PROFILES:
        raise ValueError(f"unsupported RAM-only executable profile: {name!r}")
    return PROFILES[name]


def check_remote_environment(report):
    """Validate the read-only remote prerequisite report before any upload.

    Report unavailable commands by name, instead of unexplained exit 127.
    """
    if ("MPFREE RAM SMOKE ENVIRONMENT READ ONLY" not in report
            or "END MPFREE RAM SMOKE ENVIRONMENT" not in report):
        raise RuntimeError("incomplete or unexpected remote utility report")
    details = {}
    for line in report.splitlines():
        if line.startswith(("tool.", "run_mount_type=", "factory_service_active=",
                            "factory_main_pid=", "armhf_interpreter=",
                            "shell_cpu_limit=", "shell_scheduler_policy=")):
            key, value = line.split("=", 1)
            if key in details:
                raise RuntimeError(f"duplicate prerequisite field: {key}")
            details[key] = value
    unavailable = [
        tool for tool in REQUIRED_RUNTIME_TOOLS
        if details.get("tool." + tool) != "present"
    ]
    if unavailable:
        raise RuntimeError(
            "MPC missing required RAM-smoke utilities: "
            + ", ".join(unavailable)
            + "; see read-only --ram-smoke-env report; no upload attempted"
        )
    if details.get("run_mount_type") != "tmpfs":
        raise RuntimeError(
            "MPC /run not confirmed RAM-backed tmpfs; no upload attempted"
        )
    if details.get("factory_service_active") != "active" or (
            details.get("factory_main_pid") != "present"):
        raise RuntimeError(
            "factory MPC app not confirmed active with a live MainPID; "
            "no upload attempted"
        )
    if details.get("armhf_interpreter") != "present":
        raise RuntimeError(
            "ARM hard-float interpreter missing; no upload attempted"
        )
    if details.get("shell_cpu_limit") != "present":
        raise RuntimeError(
            "POSIX shell CPU-time limit unavailable; no upload attempted"
        )
    if details.get("shell_scheduler_policy") != "0":
        raise RuntimeError(
            "SSH-launched shell is not proven SCHED_OTHER (policy 0); "
            "no upload attempted"
        )
    return (
        "PASS: /run tmpfs, 8 required utilities, stock MPC active, "
        "ARM interpreter, shell CPU limit, SCHED_OTHER; "
        "timeout/nice not required"
    )


def remote_run_script(expected_sha):
    """Only a fixed command sequence; caller cannot inject remote paths."""
    if re.fullmatch(r"[0-9a-f]{64}", expected_sha) is None:
        raise ValueError("invalid hash")
    return f"""set -eu
printf 'STAGE=CHECK_RAM_MOUNT\n'
# Assert the exact /run mount is tmpfs before creating any file.
awk '$2=="/run" && $3=="tmpfs" {{found=1}} END{{exit !found}}' /proc/mounts
printf 'STAGE=CHECK_UTILITIES\n'
for tool in mktemp cat sha256sum awk chmod rm rmdir systemctl; do
    command -v "$tool" >/dev/null
done
printf 'STAGE=CHECK_FACTORY_APP\n'
# Require the stock GUI to be active throughout the coexistence test.
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null)"
case "$pid" in ''|0|*[!0-9]*) exit 5 ;; esac
test -d "/proc/$pid"
printf 'STAGE=CREATE_TMPFS_DIR\n'
umask 077
stage="$(mktemp -d /run/mpfree-smoke.XXXXXXXX)"
cleanup() {{
    rm -f "$stage/probe" 2>/dev/null || :
    rmdir "$stage" 2>/dev/null || :
}}
trap cleanup 0
trap 'exit 124' 1 2 15
printf 'STAGE=RECEIVE_PAYLOAD\n'
cat > "$stage/probe"
chmod 0500 "$stage/probe"
printf 'STAGE=VERIFY_SHA256\n'
observed="$(sha256sum "$stage/probe" | awk '{{print $1}}')"
test "$observed" = "{expected_sha}" || {{
    printf 'SHA256_MISMATCH\\n' >&2
    exit 3
}}
printf 'MPFREE_VOLATILE_STAGE_SHA256_OK\\n'
# No core dumps or regular-file creation permitted by the child process.
ulimit -c 0
ulimit -f 0
# Kernel-enforced CPU-time cap. Our Rust self-test carries a separate
# 5-second internal watchdog thread for wall-clock overruns.
# No timeout/nice binaries are installed on this embedded factory image.
ulimit -t 3
# Refuse to run if the inherited SSH scheduling policy is real-time.
# awk reads its own proc stat after inheriting the shell's scheduler policy.
policy="$(awk '{{sub(/^.*\\) /, ""); if (NF >= 39) print $39; else print "unavailable"}}' /proc/self/stat)"
test "$policy" = "0"
printf 'STAGE=EXECUTE_RUST\n'
"$stage/probe" --self-test
printf 'STAGE=RUST_EXECUTED\n'
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
"""


def parse_results(stdout, stderr, code, profile="device-smoke"):
    """Validate device output; never infer success from a partial marker."""
    p = get_profile(profile)
    if code:
        raise RuntimeError(
            f"remote {profile} exited {code}; "
            f"last stdout={stdout[-1500:]!r}; last stderr={stderr[-800:]!r}"
        )
    lines = stdout.splitlines()
    expected_markers = (
        "MPFREE_VOLATILE_STAGE_SHA256_OK", "STAGE=RUST_EXECUTED",
        p["sentinel"],
    )
    for marker in expected_markers:
        if lines.count(marker) != 1:
            raise RuntimeError(f"missing or repeated remote {profile} marker: {marker}")
    if stderr.splitlines().count(p["watchdog"]) != 1:
        raise RuntimeError(
            f"expected on-device {profile} Rust watchdog not proven; "
            "rebuild the correct ARMv7 target executable"
        )
    # Require exactly one key=value for every expected field. A malicious or
    # stale program printing duplicate / contradictory fields must be refused.
    fields = {}
    for line in lines:
        if "=" in line:
            key, value = line.split("=", 1)
            if key == "STAGE":
                continue  # Many legitimate shell phases use STAGE=...
            if key in fields:
                raise RuntimeError(f"duplicate remote output field: {key}")
            fields[key] = value
    for key, value in p["expected"].items():
        if fields.get(key) != value:
            raise RuntimeError(
                f"unexpected {profile} result for {key}: "
                f"expected {value!r}, received {fields.get(key)!r}"
            )
    if profile == "headless":
        for key in ("nonzero_samples", "elapsed_us", "peak_rss_kib", "rss_kib"):
            value = fields.get(key)
            if value is None:
                raise RuntimeError(f"headless performance field missing: {key}")
            if key.endswith("_kib") and value == "unavailable":
                continue
            if not value.isdigit() or (key != "rss_kib" and int(value) < 1):
                raise RuntimeError(f"invalid headless performance metric {key}: {value!r}")
        if not re.fullmatch(r"[0-9a-f]{16}", fields.get("checksum_fnv1a64", "")):
            raise RuntimeError("invalid headless audio checksum")
        if int(fields["nonzero_samples"]) > 2 * int(fields["frames"]):
            raise RuntimeError("headless nonzero sample count exceeds stereo capacity")
        try:
            peak = float(fields["peak_abs"])
        except (ValueError, KeyError):
            raise RuntimeError("missing or invalid headless audio peak") from None
        if not (0 < peak <= 1):
            raise RuntimeError("headless audio peak outside normalized PCM range")
    if profile == "session":
        # A synthetic deadline miss is valid measurement DATA, not a failure
        # to execute. Do not reject nonzero counts, unlike wrong PCM.
        metrics = (
            "deadline_misses", "late_starts", "max_callback_us",
            "p99_callback_us", "max_start_lateness_us",
            "total_render_us", "wall_elapsed_us",
        )
        for metric in metrics:
            value = fields.get(metric, "")
            if not re.fullmatch(r"[0-9]+", value):
                raise RuntimeError(f"invalid M7 synthetic timing field {metric}: {value!r}")
        count = int(fields["callback_count"])
        if int(fields["deadline_misses"]) > count or int(fields["late_starts"]) > count:
            raise RuntimeError("M7 misses/late starts exceed callback count")
        if int(fields["p99_callback_us"]) > int(fields["max_callback_us"]):
            raise RuntimeError("M7 p99 duration exceeds maximum callback duration")
        if not 1_700_000 <= int(fields["wall_elapsed_us"]) < 7_000_000:
            raise RuntimeError("M7 paced session did not complete in bounded wall time")
        if int(fields["total_render_us"]) <= 0:
            raise RuntimeError("M7 measured zero render work")
    return fields


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("ip", "user", "key", "binary", "abi-script", "abi-checker"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--readelf", default="arm-linux-gnueabihf-readelf")
    parser.add_argument("--env-script", required=True)
    parser.add_argument("--profile", choices=tuple(PROFILES), default="device-smoke")
    parser.add_argument("--execute", action="store_true")
    a = parser.parse_args(argv)
    if not 1 <= a.port <= 65535:
        raise ValueError("invalid port")
    profile = get_profile(a.profile)
    binary_path = Path(a.binary)
    if binary_path.name != profile["binary"]:
        raise ValueError(
            f"refusing non-allowlisted executable {binary_path.name!r} for "
            f"{a.profile}; expected {profile['binary']!r}"
        )
    if (not binary_path.is_file() or not Path(a.abi_script).is_file()
            or not Path(a.env_script).is_file()):
        raise ValueError("expected compiled ELF or reviewed read-only survey script missing")
    if binary_path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError("binary exceeds 2 MiB volatile staging budget")
    binary = binary_path.read_bytes()
    # An old locally cross-built binary lacks its own watchdog. This must
    # fail before any MPC execution; do not infer safety from source alone.
    if profile["watchdog"].encode("ascii") not in binary:
        raise ValueError(
            f"ARMv7 {profile['binary']} binary is stale or lacks the "
            f"allowlisted watchdog; rebuild this exact target after git pull"
        )
    if profile["sentinel"].encode("ascii") not in binary:
        raise ValueError(
            f"ARMv7 {profile['binary']} is missing its version marker; "
            "rebuild the exact target executable"
        )
    digest = hashlib.sha256(binary).hexdigest()
    # Reject a file that changed between reading the upload bytes and the
    # ELF loader/symbol checks against its on-disk path.


    print("LIVE READ-ONLY ABI check (no on-device file writes).", flush=True)
    code, out, err = call_ssh(
        a.ip, a.port, a.user, a.key,
        "sh -s", Path(a.abi_script).read_bytes(), 35,
    )
    if code != 0:
        raise RuntimeError(f"read-only SSH ABI failed: {err[:800]}")
    verify_live_board(out)
    print("Exact original MPC One / az0x 5.0.8 / glibc 2.39 confirmed.", flush=True)
    print("LIVE READ-ONLY RAM runtime prerequisites (no upload).", flush=True)
    code, env_out, env_err = call_ssh(
        a.ip, a.port, a.user, a.key, "sh -s",
        Path(a.env_script).read_bytes(), 25,
    )
    if code != 0:
        raise RuntimeError(
            f"read-only utility inspection failed (exit {code}): "
            f"stdout={env_out[-800:]!r}, stderr={env_err[-500:]!r}"
        )
    print(check_remote_environment(env_out), flush=True)
    preflight_elf(Path(a.abi_checker), out, binary_path, a.readelf)
    if hashlib.sha256(binary_path.read_bytes()).hexdigest() != digest:
        raise RuntimeError("ELF changed after ABI validation; refusing upload")
    print(f"Candidate bytes={len(binary)}, SHA256={digest}", flush=True)
    if not a.execute:
        print("PREFLIGHT ONLY: nothing uploaded or executed on MPC.")
        return 0

    print("OWNER OPT-IN: temporary /run upload and bounded Rust execution.", flush=True)
    remote_command = "sh -c " + shlex.quote(remote_run_script(digest))
    code, stdout, stderr = call_ssh(
        a.ip, a.port, a.user, a.key, remote_command, binary, 18,
    )
    fields = parse_results(stdout, stderr, code, a.profile)
    print(stdout.strip())
    print(profile["watchdog"])
    print(
        f"PASS: {a.profile} ran on MPC through null-only userspace; "
        "factory devices untouched."
    )
    if a.profile == "session":
        print("M7 NULL-audio synthetic callback statistics on actual MPC:")
        for key in (
            "callback_count", "deadline_misses", "late_starts",
            "max_callback_us", "p99_callback_us",
            "max_start_lateness_us", "total_render_us", "wall_elapsed_us",
        ):
            print(f"M7_{key}={fields[key]}")
        print("CAUTION: synthetic scheduler wakeups, NOT real ACVA ALSA callback/xruns.")
    if a.profile == "headless":
        duration_seconds = int(fields["frames"]) / 44_100
        elapsed_us = int(fields["elapsed_us"])
        print(f"headless_audio_seconds={duration_seconds:.3f}")
        print(f"headless_offline_throughput_x={duration_seconds / (elapsed_us / 1_000_000):.2f}")
        print("NOTE: offline throughput is NOT an ALSA real-time deadline/xrun test.")
    print("Remote shell cleanup trap removed /run stage on normal completion.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
        print(f"FAIL CLOSED: {e}", file=sys.stderr)
        sys.exit(2)
