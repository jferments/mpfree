# USB autorun reference implementation — tested research lab, NOT a generic installer

**Status:** successfully created and independently verified for **one** owner-authorized MPC One developer setup, 2026-11-08. This preserves the exact original tooling used to obtain SSH access. It is a historical/reproducibility artifact, **not** instructions to run it on arbitrary USB drives.

## Source files committed in this repository

- [prepare_mpc_one_usb.py](../../tools/mpc-one/reference-lab/prepare_mpc_one_usb.py) — host-side USB preparation, locked to `/dev/sdb` and `/dev/sdb1`, model `USB 3.2.1 FD`, FAT32 label `USB321FD` and expected ~64 GB capacity. The deliberate identity checks are safety gates, **not** defaults other developers should copy.
- [verify_mpc_one_usb.py](../../tools/mpc-one/reference-lab/verify_mpc_one_usb.py) — independent, read-only package/checksum/public-key verification against the exact expected reference device.
- [developer-access.md](developer-access.md) — working SSH/reconnection and diagnostics.
- [USB installer design](../project/usb-installer.md) — separate future general installer design.

These two Python source files were copied **byte-for-byte** from the local scripts used in the successful session. They do not include a private SSH key, public key tied to a specific owner, vendor sample packs, extracted firmware or binary payloads. The scripts should not be run by other users until their devices have been identified and an appropriate safe general builder exists.

### Source reproducibility (Git blob hashes)

| File | Git blob SHA-1 |
| --- | --- |
| `prepare_mpc_one_usb.py` | `45721099d111608d19fe9d49a0aa1f8b06a99fdd` |
| `verify_mpc_one_usb.py` | `23d013fc8ca5acaa7ff4a01374cb601bf2d04efe` |

### Original upstream immutable autorun package

GitHub source: [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore), pinned to commit `4955a83686227690495aaad68e3ba4e39f5afaec`.

| Name | Upstream Git blob SHA-1 | Reference SHA-256 observed after USB write |
| --- | --- | --- |
| `autorun.json` | `a3b7d9123be721a15cf1e8282af81639616af26e` | `56b5a060620b4962024a2904f42cf9257a375124a18fb509cbb98f205293491a` |
| `autorun.check` | `6f7cb31d089bc3933d4111f6dc586b8fe4fbd679` | `85e1d195f740d7ec9303f1071b26fca64aeaece806487e6a3a672cd52cc289c8` |
| `tkgl_static_script.sh` | `da5bfb147f4b0f6c2250476c2e52c400f1b134a8` | `b33f22af5e12e3629b0875686403b3f87545d0d781c42e235064dc7e9ea0bbe9` |

The three upstream files are **not edited** in the package. Their exact bytes are essential to the observed validation mechanism. The separate `tkgl_sub_script.sh` is the only customized script; it starts an existing key-only OpenSSH server in `/run`. The original USB preparation downloaded immutable inputs and independently checked their identity before writing.

## Reference USB safety workflow (already completed)

~~~text
Workstation detection:
    /dev/sdb    USB transport, expected model and size
    /dev/sdb1   FAT32, label USB321FD
Preparation dry-run: PASS
Explicit target confirmation: WRITE /dev/sdb1
No repartition, formatting or firmware flash
Immutable upstream source file checks: PASS
Custom payload shell syntax and reviewed SHA-256: PASS
Dedicated owner public key matches local private-key pair: PASS
No suspicious update image in USB root: PASS
Verified USB unmounted before insertion: PASS
MPC autorun launches key-only temporary SSH: PASS on this device
~~~

The temporary SSH configuration used TCP port `2222`, root public-key login, and RAM-backed runtime under `/run/mpc-one-autorun-ssh`. It did not write factory rootfs or alter boot partitions. (A *later*, separate persistent-SSH setup did write the rootfs; see [device-modifications.md](../hardware/device-modifications.md).) It **did** write the approved autorun folder to the research USB and could record status/fingerprint there.

## Why not distribute this as the end-user installer?

- `/dev/sdb` is only a Linux device name, not stable hardware identity; blindly assuming it is the correct disk could destroy unrelated data. These scripts refuse anything but the explicitly verified lab device.
- A particular public firmware version and installed SSH daemon were observed **on one unit only**. Other firmware may reject the autorun mechanism.
- The end-user product needs supported model/firmware gates, rollback, verified recovery, cryptographic release signing, portable user-owned content import, and zero-shared-key provisioning.
- The factory USB mount uses `noexec` semantics, so a future compiled MPFree binary cannot be assumed executable straight from FAT32.
- The scripts are research prototypes, not audited production installer software.

For future generalization, replace the hardcoded USB identity with an enumerated hardware-confirmation workflow and refusal-on-ambiguity; preserve unchanged upstream static files; generate *per-user/per-device* authentication; never require a public repo to contain private credentials. Keep system partition writes disabled until recovery is proven.

## Next steps

1. Run the [repository-hosted read-only inventory](../../tools/mpc-one/collect-over-ssh.sh) and publish redacted findings with original evidence commands.
2. Document tested boot, storage and driver interfaces separately by model/firmware.
3. Prove restoration independently before changing persistent partitions or services.
4. Design the one-USB self-contained bootstrap and installer with an explicit compatibility table and no SSH dependency for normal users.
