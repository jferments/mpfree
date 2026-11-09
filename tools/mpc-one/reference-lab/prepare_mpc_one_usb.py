#!/usr/bin/env python3
"""Prepare an existing MPC One autorun FAT32 USB partition without formatting it.

Only writes a newly-created 'autorun' directory on a carefully verified USB
partition, and a dedicated SSH key under the user's ~/.ssh directory if absent.
DO NOT run as root. GitHub upstream static files remain byte-for-byte unchanged.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import urllib.request

REPO = 'TheKikGen/MPC-LiveXplore'
COMMIT = '4955a83686227690495aaad68e3ba4e39f5afaec'
# Git object IDs obtained from upstream repository's master tree, Nov 8, 2026.
UPSTREAM = {
    'autorun.json': 'a3b7d9123be721a15cf1e8282af81639616af26e',
    'autorun.check': '6f7cb31d089bc3933d4111f6dc586b8fe4fbd679',
    'tkgl_static_script.sh': 'da5bfb147f4b0f6c2250476c2e52c400f1b134a8',
}
EXPECTED_SHA256 = {
    'autorun.json': '56b5a060620b4962024a2904f42cf9257a375124a18fb509cbb98f205293491a',
    'autorun.check': '85e1d195f740d7ec9303f1071b26fca64aeaece806487e6a3a672cd52cc289c8',
    'tkgl_static_script.sh': 'b33f22af5e12e3629b0875686403b3f87545d0d781c42e235064dc7e9ea0bbe9',
}
EXPECTED_CHECK = b'926b161aa9916041002b3f0bbb09dcc0ac0993e69fb475390de5edfccf17ac90'

# This is the only custom code run on the MPC. It does not modify its firmware,
# bootloader, existing SSH config, persistent /etc, or installed services.
PAYLOAD = r'''#!/bin/sh
# MPC One: temporary, key-only OpenSSH on TCP 2222.
# All runtime files are under /run (tmpfs); logs also copied to this USB.
# There is intentionally no remount, update, systemctl, passwd, or firmware write.
set -u
umask 077
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH

# The upstream static script provides SCRIPT_DIR when sourcing this file.
USB_DIR=${SCRIPT_DIR:-}
case "$USB_DIR" in
  /*) ;;
  *) echo "MPC-SSH: unknown USB directory; aborting"; exit 1 ;;
esac
STATUS="$USB_DIR/mpc_ssh_status.txt"
report() {
    printf '%s\n' "MPC-SSH: $*"
    # USB storage may be read-only; failure to write a status must not be fatal.
    { printf '%s\n' "$*" >> "$STATUS"; } 2>/dev/null || :
}
fail() { report "ERROR: $*"; exit 1; }

[ "$(id -u)" = 0 ] || fail 'Not running as root'
[ -x /usr/bin/MPC ] || fail '/usr/bin/MPC not found: wrong or unsupported device'
[ -r "$USB_DIR/mpc_authorized_keys.pub" ] || fail 'Missing authorized public key'
# Refuse to put even temporary SSH files on an unknown/persistent filesystem.
awk '$2 == "/run" && $3 == "tmpfs" { ok=1 } END { exit !ok }' /proc/mounts || \
    fail '/run is not a verified tmpfs; refusing to write'
STATE=/run/mpc-one-autorun-ssh
mkdir -p "$STATE" || fail 'Cannot create runtime directory'
chmod 700 "$STATE" || fail 'Cannot secure runtime directory'

# Avoid spawning duplicate daemons if the user executes autorun twice.
if [ -s "$STATE/sshd.pid" ]; then
    oldpid=$(cat "$STATE/sshd.pid" 2>/dev/null) || oldpid=''
    case "$oldpid" in
      *[!0-9]*|'') ;;
      *) if kill -0 "$oldpid" 2>/dev/null; then
             report 'Existing SSH daemon appears to be running on port 2222'
             exit 0
         fi ;;
    esac
fi

SSHD=''
for candidate in /usr/sbin/sshd /sbin/sshd /usr/bin/sshd; do
    if [ -x "$candidate" ]; then SSHD="$candidate"; break; fi
done
[ -n "$SSHD" ] || fail 'No built-in OpenSSH sshd found; no installation attempted'
KEYGEN=''
for candidate in /usr/bin/ssh-keygen /bin/ssh-keygen; do
    if [ -x "$candidate" ]; then KEYGEN="$candidate"; break; fi
done
[ -n "$KEYGEN" ] || fail 'No built-in ssh-keygen found; no installation attempted'

cp "$USB_DIR/mpc_authorized_keys.pub" "$STATE/authorized_keys" || \
    fail 'Failed to copy authorized key to temporary memory'
chmod 600 "$STATE/authorized_keys" || fail 'Could not secure authorized key'
if [ ! -s "$STATE/host_ed25519" ]; then
    "$KEYGEN" -q -t ed25519 -N '' -f "$STATE/host_ed25519" >/dev/null 2>&1 || \
        fail 'Could not generate temporary Ed25519 host key'
fi
chmod 600 "$STATE/host_ed25519"
# Some OpenSSH builds require /run/sshd for privilege separation.
mkdir -p /run/sshd || fail 'Could not create /run/sshd'
chmod 755 /run/sshd
cat > "$STATE/sshd_config" <<EOF
Port 2222
AddressFamily inet
ListenAddress 0.0.0.0
HostKey $STATE/host_ed25519
PidFile $STATE/sshd.pid
AuthorizedKeysFile $STATE/authorized_keys
PermitRootLogin prohibit-password
PubkeyAuthentication yes
PasswordAuthentication no
PermitEmptyPasswords no
ChallengeResponseAuthentication no
UsePAM no
AllowUsers root
StrictModes yes
LogLevel VERBOSE
X11Forwarding no
AllowTcpForwarding no
AllowAgentForwarding no
PermitTunnel no
Subsystem sftp internal-sftp
EOF
"$SSHD" -t -f "$STATE/sshd_config" > "$STATE/config_test.log" 2>&1 || {
    report 'ERROR: SSH config validation failed; details follow:'
    while IFS= read -r ln; do report "$ln"; done < "$STATE/config_test.log"
    exit 1
}
"$SSHD" -f "$STATE/sshd_config" -E "$STATE/daemon.log" || {
    report 'ERROR: SSH daemon failed to start; details follow:'
    if [ -r "$STATE/daemon.log" ]; then
        while IFS= read -r ln; do report "$ln"; done < "$STATE/daemon.log"
    fi
    exit 1
}
sleep 1
if [ -r "$STATE/sshd.pid" ]; then
    pid=$(cat "$STATE/sshd.pid")
    if kill -0 "$pid" 2>/dev/null; then
        report 'SUCCESS: temporary, key-only SSH server on TCP port 2222'
        report 'Connect: ssh -p 2222 -i <your-private-key> root@<MPC_IP>'
        "$KEYGEN" -lf "$STATE/host_ed25519.pub" > "$USB_DIR/mpc_ssh_host_fingerprint.txt" 2>/dev/null || :
        exit 0
    fi
fi
fail 'SSH daemon not confirmed running; see /run/mpc-one-autorun-ssh/daemon.log'
'''

class GuardError(RuntimeError):
    pass


def run(*argv: str) -> str:
    cp = subprocess.run(argv, check=True, text=True, stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE)
    return cp.stdout.strip()


def blob_sha(data: bytes) -> str:
    return hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()


def check_device(device: str) -> tuple[str, str]:
    if os.geteuid() == 0:
        raise GuardError('Do not run as root or via sudo. USB mounting uses udisksctl.')
    device = os.path.realpath(device)
    if device != '/dev/sdb':
        raise GuardError('Refusing device other than /dev/sdb; recheck lsblk first.')
    info = json.loads(run('lsblk', '--json', '--bytes', '--output',
                          'NAME,PATH,TYPE,TRAN,MODEL,SIZE,FSTYPE,FSVER,LABEL,MOUNTPOINTS',
                          device))
    blocks = info['blockdevices']
    if len(blocks) != 1:
        raise GuardError('Unexpected device tree.')
    disk = blocks[0]
    if (disk.get('type') != 'disk' or disk.get('tran') != 'usb'
            or (disk.get('model') or '').strip() != 'USB 3.2.1 FD'
            or not 55_000_000_000 <= int(disk.get('size') or 0) <= 70_000_000_000):
        raise GuardError(f'Disk identity mismatch. Got: {disk}')
    children = disk.get('children') or []
    if len(children) != 1:
        raise GuardError('Expected exactly one partition; refusing to modify multi-partition drive.')
    part = children[0]
    if (part.get('path') != '/dev/sdb1' or part.get('type') != 'part'
            or part.get('fstype') != 'vfat' or part.get('label') != 'USB321FD'):
        raise GuardError(f'Partition identity mismatch. Got: {part}')
    version = (part.get('fsver') or '').upper().strip()
    if version != 'FAT32':
        raise GuardError(f'Existing filesystem not confirmed FAT32 (found {version!r}). '
                         'Do not format automatically.')
    if not stat.S_ISBLK(os.stat(device).st_mode) or not stat.S_ISBLK(os.stat('/dev/sdb1').st_mode):
        raise GuardError('Paths are not block devices.')
    return device, '/dev/sdb1'


def mounting(partition: str) -> Path:
    # findmnt returns an existing mount if present; otherwise udisksctl mounts
    # it as the current user without a privileged manual mount command.
    mounts = run('findmnt', '-rn', '-o', 'TARGET', '--source', partition) if subprocess.run(
        ['findmnt','-rn','-o','TARGET','--source',partition], capture_output=True).returncode == 0 else ''
    if not mounts:
        print('USB partition is not mounted. Asking udisksctl to mount it...')
        run('udisksctl', 'mount', '-b', partition)
        mounts = run('findmnt', '-rn', '-o', 'TARGET', '--source', partition)
    targets = mounts.splitlines()
    if len(targets) != 1:
        raise GuardError('Partition has zero or multiple mounts; refusing.')
    path = Path(targets[0]).resolve(strict=True)
    if path == Path('/'):
        raise GuardError('Refusing root filesystem as mountpoint.')
    if os.stat(path).st_dev != os.stat(partition).st_rdev:
        raise GuardError('Mountpoint does not belong to expected USB block device.')
    fs = run('findmnt', '-rn', '-o', 'FSTYPE', '--target', str(path))
    opts = run('findmnt', '-rn', '-o', 'OPTIONS', '--target', str(path))
    if fs != 'vfat' or 'rw' not in opts.split(','):
        raise GuardError(f'USB mount must be writable vfat (found {fs}, {opts}).')
    return path


def get_key(pub_override: str | None) -> bytes:
    if pub_override:
        pub = Path(pub_override).expanduser()
    else:
        priv = Path.home() / '.ssh' / 'mpc_one_autorun_ed25519'
        pub = Path(str(priv) + '.pub')
        if not pub.exists():
            if priv.exists():
                raise GuardError(f'{priv} exists but public key absent. Refusing overwrite.')
            priv.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            print(f'Generating a dedicated SSH key: {priv}')
            print('ssh-keygen will prompt you for an optional passphrase.')
            subprocess.run(['ssh-keygen', '-t', 'ed25519', '-f', str(priv),
                            '-C', 'mpc-one-usb-autorun'], check=True)
    if not pub.is_file() or pub.is_symlink():
        raise GuardError(f'Public key not a regular file: {pub}')
    data = pub.read_bytes().strip() + b'\n'
    fields = data.strip().split()
    if not (len(fields) in (2, 3) and fields[0] == b'ssh-ed25519'):
        raise GuardError('Requires a single ssh-ed25519 public key (no authorized_keys options).')
    try:
        raw = base64.b64decode(fields[1], validate=True)
        if len(raw) < 40 or not raw.startswith(b'\0\0\0\x0bssh-ed25519'):
            raise ValueError('incorrect key format')
    except Exception as exc:
        raise GuardError(f'Invalid OpenSSH public key: {exc}') from exc
    run('ssh-keygen', '-lf', str(pub))
    return data


def download_upstream() -> dict[str, bytes]:
    result = {}
    for name, sha in UPSTREAM.items():
        url = f'https://raw.githubusercontent.com/{REPO}/{COMMIT}/MPC/autorun/{name}'
        with urllib.request.urlopen(urllib.request.Request(url,
                  headers={'User-Agent': 'mpc-one-safe-usb-preparation/1.0'}), timeout=25) as r:
            data = r.read(50000)
        if blob_sha(data) != sha or hashlib.sha256(data).hexdigest() != EXPECTED_SHA256[name]:
            raise GuardError(f'UPSTREAM HASH MISMATCH for {name}; refusing to use it.')
        result[name] = data
    if result['autorun.check'] != EXPECTED_CHECK:
        raise GuardError('Upstream autorun.check did not match author-documented bytes.')
    manifest = json.loads(result['autorun.json'])
    scripts = manifest['value0']['scripts']
    if len(scripts) != 1 or scripts[0]['path'] != 'tkgl_static_script.sh' or not scripts[0]['askQuestion']:
        raise GuardError('Upstream autorun manifest differs from expected behavior.')
    if b'. "$TKGL_SUB_SCRIPT_NAME"' not in result['tkgl_static_script.sh']:
        raise GuardError('Static script no longer sources the expected subscript.')
    return result


def stage_package(mount: Path, upstream: dict[str, bytes], key: bytes) -> None:
    dest = mount / 'autorun'
    if os.path.lexists(dest):
        raise GuardError(f'{dest} ALREADY EXISTS; will not overwrite user content.')
    stage = Path(tempfile.mkdtemp(prefix='.mpc-one-staging-', dir=mount))
    try:
        folder = stage / 'autorun'
        folder.mkdir()
        files = dict(upstream)
        files['tkgl_sub_script.sh'] = PAYLOAD.encode('ascii')
        files['mpc_authorized_keys.pub'] = key
        for name, data in files.items():
            (folder / name).write_bytes(data)
        # Round-trip storage check before making the folder visible as 'autorun'.
        for name, data in files.items():
            if (folder / name).read_bytes() != data:
                raise GuardError(f'USB readback mismatch for {name}.')
        os.sync()
        if os.path.lexists(dest):
            raise GuardError('autorun folder appeared during staging; refusing overwrite.')
        folder.rename(dest)
        os.sync()
        for name, data in files.items():
            if (dest / name).read_bytes() != data:
                raise GuardError(f'USB final readback mismatch for {name}.')
        print('\nSUCCESS: Created', dest)
        for name in sorted(files):
            print(' ', name, 'sha256:', hashlib.sha256(files[name]).hexdigest())
    finally:
        if stage.exists():
            shutil.rmtree(stage)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--device', default='/dev/sdb', help='Must be /dev/sdb, no automatic guesses')
    parser.add_argument('--dry-run', action='store_true', help='Check USB identity only; do not write')
    parser.add_argument('--public-key', help='Use an existing ED25519 public key instead of dedicated key')
    args = parser.parse_args()
    try:
        device, part = check_device(args.device)
        print(f'Identity verified: {device}: USB 3.2.1 FD, FAT32, USB321FD, /dev/sdb1')
        if args.dry_run:
            print('DRY RUN complete. Nothing was written or mounted.')
            return 0
        mount = mounting(part)
        print('USB mounted at:', mount)
        if os.path.lexists(mount / 'autorun'):
            raise GuardError('USB already has an autorun entry; refusing to overwrite it.')
        for item in mount.iterdir():
            if item.is_file() and item.name.lower().endswith(('.img', '.upd', '.upg', '.bin')):
                raise GuardError(f'USB root contains a firmware-looking file: {item.name}. '
                                 'Review/remove it manually before staging.')
        print('Downloading original autorun files pinned to upstream commit', COMMIT)
        upstream = download_upstream()
        key = get_key(args.public_key)
        print('\nWRITE TARGET:', mount / 'autorun')
        print('No formatting, repartitioning, firmware updates, or other drive writes.')
        answer = input('To write this USB, type exactly: WRITE /dev/sdb1\n> ')
        if answer != 'WRITE /dev/sdb1':
            raise GuardError('Confirmation not given; no USB payload written.')
        # Avoid TOCTOU / accidental disconnection during download/key creation.
        check_device(device)
        if os.stat(mount).st_dev != os.stat(part).st_rdev:
            raise GuardError('Device was remounted or replaced; abort.')
        stage_package(mount, upstream, key)
        print('\nRun verify_mpc_one_usb.py next, before using this stick.')
        print('After verification: sync; udisksctl unmount -b /dev/sdb1')
        return 0
    except (GuardError, OSError, ValueError, subprocess.CalledProcessError) as err:
        print('\nABORTED:', err, file=sys.stderr)
        return 1

if __name__ == '__main__':
    sys.exit(main())