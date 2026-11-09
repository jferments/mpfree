#!/usr/bin/env python3
"""Independent, READ-ONLY verification of the MPC One autorun USB package.

No firmware changes, filesystem formatting, automatic mounts, or USB writes.
This verifies data layout and checksums, not firmware compatibility or real SSH.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys

STATIC_SHA1 = {
    'autorun.json': 'a3b7d9123be721a15cf1e8282af81639616af26e',
    'autorun.check': '6f7cb31d089bc3933d4111f6dc586b8fe4fbd679',
    'tkgl_static_script.sh': 'da5bfb147f4b0f6c2250476c2e52c400f1b134a8',
}
STATIC_SHA256 = {
    'autorun.json': '56b5a060620b4962024a2904f42cf9257a375124a18fb509cbb98f205293491a',
    'autorun.check': '85e1d195f740d7ec9303f1071b26fca64aeaece806487e6a3a672cd52cc289c8',
    'tkgl_static_script.sh': 'b33f22af5e12e3629b0875686403b3f87545d0d781c42e235064dc7e9ea0bbe9',
}
PAYLOAD_SHA256 = '9ab1a59e26c4475a4bd30bbd7f994e821c0ffbe6954d3f85275984c445c41107'
CHECK_BYTES = b'926b161aa9916041002b3f0bbb09dcc0ac0993e69fb475390de5edfccf17ac90'
BASE_FILES = set(STATIC_SHA1) | {'tkgl_sub_script.sh', 'mpc_authorized_keys.pub'}
OPTIONAL_RUNTIME = {'mpc_ssh_status.txt', 'mpc_ssh_host_fingerprint.txt'}

class Invalid(RuntimeError):
    pass


def run(*args: str) -> str:
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout.strip()


def git_blob_hash(contents: bytes) -> str:
    return hashlib.sha1(b'blob ' + str(len(contents)).encode() + b'\x00' + contents).hexdigest()


def verify_usb_device() -> Path:
    device = '/dev/sdb'
    if os.path.realpath(device) != device:
        raise Invalid('/dev/sdb is not canonical')
    data = json.loads(run('lsblk','--json','--bytes','--output',
          'NAME,PATH,TYPE,TRAN,MODEL,SIZE,FSTYPE,FSVER,LABEL,MOUNTPOINTS',device))
    drives = data.get('blockdevices', [])
    if len(drives) != 1:
        raise Invalid('Unexpected number of disk objects')
    disk = drives[0]
    if (disk.get('type') != 'disk' or disk.get('tran') != 'usb'
            or (disk.get('model') or '').strip() != 'USB 3.2.1 FD'
            or not 55_000_000_000 <= int(disk.get('size') or 0) <= 70_000_000_000):
        raise Invalid(f'Unexpected USB device: {disk}')
    kids = disk.get('children') or []
    if len(kids) != 1:
        raise Invalid('Device is not a single-partition drive')
    p = kids[0]
    if not (p.get('path') == '/dev/sdb1' and p.get('type') == 'part'
            and p.get('fstype') == 'vfat' and (p.get('fsver') or '').upper() == 'FAT32'
            and p.get('label') == 'USB321FD'):
        raise Invalid(f'Partition mismatch: {p}')
    mount_lines = run('findmnt', '-rn','-o','TARGET','--source','/dev/sdb1').splitlines()
    if len(mount_lines) != 1:
        raise Invalid('Expected one mounted filesystem for /dev/sdb1; mount first.')
    mount = Path(mount_lines[0]).resolve(strict=True)
    if mount == Path('/') or os.stat(mount).st_dev != os.stat('/dev/sdb1').st_rdev:
        raise Invalid('Mountpoint does not belong to /dev/sdb1')
    if run('findmnt','-rn','-o','FSTYPE','--target',str(mount)) != 'vfat':
        raise Invalid('USB mount is not vfat')
    print('[PASS] Device and mount identity: /dev/sdb1, FAT32, correct USB drive')
    return mount


def check_pubkey(blob: bytes) -> None:
    if b'\r' in blob or not blob.endswith(b'\n'):
        raise Invalid('Public key has incorrect line endings')
    lines = blob.strip().splitlines()
    if len(lines) != 1:
        raise Invalid('Expected exactly one public key')
    fields = lines[0].split()
    if len(fields) not in (2, 3) or fields[0] != b'ssh-ed25519':
        raise Invalid('Not a bare Ed25519 SSH public key')
    try:
        raw = base64.b64decode(fields[1], validate=True)
    except Exception as exc:
        raise Invalid(f'Public key invalid base64: {exc}') from exc
    if not raw.startswith(b'\x00\x00\x00\x0bssh-ed25519') or len(raw) < 40:
        raise Invalid('SSH public key binary format invalid')


def check_file(folder: Path, name: str) -> bytes:
    path = folder / name
    if path.is_symlink() or not path.is_file():
        raise Invalid(f'Not a regular file: {name}')
    if path.stat().st_dev != folder.stat().st_dev:
        raise Invalid(f'File on unexpected filesystem: {name}')
    data = path.read_bytes()
    if len(data) == 0 or len(data) > 100_000:
        raise Invalid(f'Unexpected file size: {name}')
    return data


def verify_tree(root: Path, pubkey: Path) -> None:
    autorun = root / 'autorun'
    if autorun.is_symlink() or not autorun.is_dir() or autorun.stat().st_dev != root.stat().st_dev:
        raise Invalid('Missing or unsafe /autorun directory')
    names = {entry.name for entry in autorun.iterdir()}
    if not BASE_FILES.issubset(names):
        raise Invalid('Missing files: ' + ', '.join(sorted(BASE_FILES - names)))
    if names - BASE_FILES - OPTIONAL_RUNTIME:
        raise Invalid('Unexpected files in autorun: ' + ', '.join(sorted(names - BASE_FILES - OPTIONAL_RUNTIME)))
    for name, expect in STATIC_SHA1.items():
        got = git_blob_hash(check_file(autorun, name))
        if got != expect or hashlib.sha256(check_file(autorun, name)).hexdigest() != STATIC_SHA256[name]:
            raise Invalid(f'Original file {name} does NOT match verified upstream hashes! {got}')
        print(f'[PASS] Original immutable file is byte-perfect: {name} ({expect})')
    manifest = json.loads(check_file(autorun, 'autorun.json'))
    scripts = manifest.get('value0', {}).get('scripts', [])
    if (len(scripts) != 1 or scripts[0].get('path') != 'tkgl_static_script.sh'
            or scripts[0].get('askQuestion') is not True
            or manifest.get('value0', {}).get('files') != []):
        raise Invalid('Unsafe/unexpected autorun.json manifest contents')
    print('[PASS] Autorun manifest asks before executing only the signed static script')
    if check_file(autorun, 'autorun.check') != CHECK_BYTES:
        raise Invalid('autorun.check validation bytes incorrect')
    static = check_file(autorun, 'tkgl_static_script.sh')
    if b'. "$TKGL_SUB_SCRIPT_NAME"' not in static:
        raise Invalid('Upstream static script no longer calls the expected subscript')
    dynamic = check_file(autorun, 'tkgl_sub_script.sh')
    if hashlib.sha256(dynamic).hexdigest() != PAYLOAD_SHA256:
        raise Invalid('Custom payload SHA-256 differs from the reviewed safe script')
    # A syntax-only check: this does not execute the code.
    for name in ('tkgl_static_script.sh', 'tkgl_sub_script.sh'):
        result = subprocess.run(['dash', '-n', str(autorun / name)], capture_output=True, text=True)
        if result.returncode != 0:
            raise Invalid(f'Shell syntax failure in {name}: {result.stderr.strip()}')
    print('[PASS] Custom payload matches reviewed SHA-256; shell syntax is valid')
    on_usb = check_file(autorun, 'mpc_authorized_keys.pub')
    check_pubkey(on_usb)
    if not pubkey.is_file() or pubkey.is_symlink():
        raise Invalid(f'Cannot verify local public key: {pubkey}')
    if on_usb != pubkey.read_bytes().strip() + b'\n':
        raise Invalid('USB public key does not match local SSH public key!')
    print('[PASS] Public SSH key matches workstation key exactly')
    for name in OPTIONAL_RUNTIME & names:
        check_file(autorun, name)
    print('[PASS] Autorun contains no unknown executable files')
    for entry in root.iterdir():
        if entry.is_file() and entry.name.lower().endswith(('.img','.upd','.upg','.bin')):
            raise Invalid(f'Firmware-looking file at USB root: {entry.name}. Remove/review manually.')
    print('[PASS] No firmware-looking update files present at USB root')


def main() -> int:
    pa = argparse.ArgumentParser(description=__doc__)
    pa.add_argument('--public-key', default='~/.ssh/mpc_one_autorun_ed25519.pub',
                    help='Workstation public key used when preparing the stick')
    args = pa.parse_args()
    try:
        mount = verify_usb_device()
        verify_tree(mount, Path(args.public_key).expanduser())
        print('\nALL USB-SIDE CHECKS PASSED.')
        print('Remaining unverified: MPC firmware compatibility, presence of sshd and actual execution.')
        print('Do not run autorun on unverified/newer firmware.')
        return 0
    except (Invalid, OSError, ValueError, KeyError, subprocess.CalledProcessError) as e:
        print('\nVERIFICATION FAILED:', e, file=sys.stderr)
        return 1

if __name__ == '__main__':
    sys.exit(main())
