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
policy="$(awk '{sub(/^.*\) /, ""); if (NF >= 39) print $39; else print "unavailable"}' /proc/self/stat)"
test "$policy" = "0"
printf 'STAGE=EXECUTE_RUST\n'
"$stage/probe" --self-test
printf 'STAGE=RUST_EXECUTED\n'
test "$(systemctl is-active acvs.service 2>/dev/null)" = "active"
"""


def parse_results(stdout, stderr, code):
    if code:
        raise RuntimeError(
            f"remote smoke exited {code}; "
            f"last stdout={stdout[-1500:]!r}; last stderr={stderr[-800:]!r}"
        )
    for marker in ("MPFREE_VOLATILE_STAGE_SHA256_OK", SMOKE_SENTINEL,
                   "status=PASS", "arch=arm", "os=linux",
                   "frames=4096", "beats=1"):
        if marker not in stdout:
            raise RuntimeError(f"missing expected remote result: {marker}")
    if RUST_WATCHDOG_MARKER not in stderr:
        raise RuntimeError(
            "no evidence the on-device Rust watchdog armed; "
            "rebuild ARMv7 ELF before any retest"
        )


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("ip", "user", "key", "binary", "abi-script", "abi-checker"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--readelf", default="arm-linux-gnueabihf-readelf")
    parser.add_argument("--env-script", required=True)
    parser.add_argument("--execute", action="store_true")
    a = parser.parse_args(argv)
    if not 1 <= a.port <= 65535:
        raise ValueError("invalid port")
    binary_path = Path(a.binary)
    if (not binary_path.is_file() or not Path(a.abi_script).is_file()
            or not Path(a.env_script).is_file()):
        raise ValueError("expected compiled ELF or reviewed read-only survey script missing")
    if binary_path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError("binary exceeds 2 MiB volatile staging budget")
    binary = binary_path.read_bytes()
    # An old locally cross-built binary lacks its own watchdog. This must
    # fail before any MPC execution; do not infer safety from source alone.
    if RUST_WATCHDOG_MARKER.encode("ascii") not in binary:
        raise ValueError(
            "ARMv7 binary is stale and lacks Rust self-watchdog; "
            "rebuild mpfree-device-smoke after git pull"
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
    parse_results(stdout, stderr, code)
    print(stdout.strip())
    print(RUST_WATCHDOG_MARKER)
    print("PASS: first on-target Rust sample computations; factory devices untouched.")
    print("Remote shell cleanup trap removed /run stage on normal completion.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
        print(f"FAIL CLOSED: {e}", file=sys.stderr)
        sys.exit(2)
