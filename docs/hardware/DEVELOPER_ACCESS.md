# MPC One developer access: reproducible SSH and safety runbook

**Scope:** owner-authorized research on one Akai MPC One running reported factory firmware 3.6.0. The commands in the **reference lab** section are exact for this already-established setup; the same addresses, SSH key and firmware assumptions will **not** apply to another device.

## Established access (2026-11-08)

USB autorun worked with unchanged files from [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore), pinned to commit `4955a83686227690495aaad68e3ba4e39f5afaec`:

~~~text
USB FAT32 root/
└── autorun/
    ├── autorun.json           upstream IMMUTABLE
    ├── autorun.check          upstream IMMUTABLE
    ├── tkgl_static_script.sh  upstream IMMUTABLE
    ├── tkgl_sub_script.sh     locally authored variable payload
    └── mpc_authorized_keys.pub
~~~

Upstream files must remain **byte-identical**; editing the manifest or primary script breaks the package's validation mechanism. The previous USB-builder validated Git blob IDs and SHA-256 hashes before writing, and an **independent** verifier checked all five on-USB files. The observed USB was a 57.7 GiB FAT32 partition with label `USB321FD`. No format, repartition or firmware flash was used.

This bootstrap temporarily launched the MPC's **existing** OpenSSH 9.6 server with:
- root login restricted to public-key authentication (passwords disabled);
- temporary daemon state under `/run/mpc-one-autorun-ssh`;
- port `2222`, not port 22;
- temporary server host key (expect change after reboot);
- no deliberate write to internal system partitions.

It may write diagnostic status/fingerprint files on the removable USB. The authenticated shell itself may cause routine logging by the factory OS.

### Reference lab network (not general defaults)

| Property | Value |
| --- | --- |
| Linux workstation | `ML-tower` |
| Wired NIC | `eno1np0` |
| NetworkManager connection | `mpc-direct` |
| Workstation IPv4 | `10.42.0.1/24` |
| MPC IPv4 | `10.42.0.141/24` |
| MPC NIC | `end0` |
| SSH port | `2222` |
| SSH login | `root` |
| Workstation private key | `/home/jesse/.ssh/mpc_one_autorun_ed25519` |

The current owner verified MPC L2 identity using `ip neigh`. Never treat an IP or MAC observed on this unit as a universal Akai factory address. Avoid committing machine identities into public test fixtures.

## Safe re-connection commands — exact reference lab

Run on the workstation, **not** in the MPC shell.

~~~sh
nmcli device status
ip -4 addr show dev eno1np0
ping -I eno1np0 -c 3 10.42.0.141
ip neigh show 10.42.0.141 dev eno1np0

ssh -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes \
  -i /home/jesse/.ssh/mpc_one_autorun_ed25519 \
  -p 2222 root@10.42.0.141
~~~

The host-key fingerprint recorded on the initial session was verified as a **new** key, not as a permanently provisioned vendor key. On reboot, the temporary key is regenerated. When SSH warns that the known host key changed, **do not disable host-key checking**. Validate the USB-side `mpc_ssh_host_fingerprint.txt` first, and only then remove the stale specific `[10.42.0.141]:2222` known-host entry if justified.

If a GUI SSH askpass application misparses interactive host-key or key passphrase prompts, retry from a real terminal with `ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519`. Do not disable key verification to work around askpass problems.

### First, confirm safety boundaries

Once logged in, the following are read-only:

~~~sh
id
uname -a
cat /etc/os-release
mount
ip addr
cat /proc/partitions
cat /proc/asound/cards
cat /proc/bus/input/devices
~~~

Never use `mount -o remount,rw /`, `fw_setenv`, `flashcp`, `dd of=/dev/mmcblk...`, `systemctl disable`, or factory update flows during inventory.

### Execute the automated read-only inventory

This repository contains:
- [collect-over-ssh.sh](../../tools/mpc-one/collect-over-ssh.sh): host-side network/key safety checks, SSH transport and local SHA-256 report;
- [read-only-inventory.sh](../../tools/mpc-one/read-only-inventory.sh): portable POSIX `sh` program sent **via SSH stdin**, never installed on the MPC.

From a clone of this branch:

~~~sh
bash tools/mpc-one/collect-over-ssh.sh
~~~

The defaults match the **reference lab** above. For another MPC, supply explicit, trusted environment settings and verify them independently before collecting data:

~~~sh
MPC_IP=<OWN_DEVICE_IP> MPC_IFACE=<OWN_WIRED_NIC> \
MPC_HOST_IP_CIDR=<OWN_HOST_CIDR> MPC_EXPECT_MAC=<OWN_MPC_MAC> \
MPC_KEY=<OWN_PRIVATE_KEY_PATH> bash tools/mpc-one/collect-over-ssh.sh
~~~

Reports are created under the workstation directory `research/` in the checked-out repository (excluded by `.gitignore`). Only the host is written by the wrapper. Never redirect output to the MPC's persistent `/data`. The collector has a hard timeout and requests no TTY. It does not open or transmit to audio/MIDI device nodes, stop services, remount storage or install binaries.

**Warning:** `research/` reports may contain MAC addresses, filesystem paths, serials and device identifiers. Review them before uploading, attaching to a chat, or proposing documentation updates.

## Evidence capture and attribution policy

Every useful reverse-engineering claim should include:
1. MPC hardware variant and reported firmware version.
2. Kernel/distribution versions (`uname -a` and `/etc/os-release`).
3. Exact harmless collection command.
4. Date and scope of the observation.
5. Raw excerpt with identifying/secret details redacted.
6. OBSERVED/DERIVED/UNVERIFIED confidence label.
7. Source, such as an owner-run device probe, upstream public repository or independently created experiment.

Maintain separate maps for **storage/boot**, **CPU/device tree**, **audio**, **MIDI/controllers/LEDs**, **display/touch**, **USB/network**, **factory process startup**, and **content formats**. Do not generalize from earlier MPC Live firmware or unrelated boards without reproducing observations.

## Failure handling

- **Ping fails:** verify `mpc-direct` is connected on `eno1np0`; do not create an overlapping NetworkManager connection.
- **Collector says `Device identity mismatch: ... observed <none>`:** This is **not** proof of a different MAC. It means the host could not resolve a complete ARP/neighbor entry. An earlier version of the script read only the fifth whitespace field of a single `ip neigh` lookup, causing a false-negative on a transient/missing record. The corrected collector retries ICMP, scans `lladdr` by name, and checks both netlink and `/proc/net/arp`. It fails closed if there is no verified MAC or the two sources conflict. It does **not** disable the security check. Diagnostic commands on the workstation:

  ~~~sh
  ping -I eno1np0 -c 3 10.42.0.141
  ip -4 neigh show to 10.42.0.141 dev eno1np0
  cat /proc/net/arp
  ~~~

  Use the current development-branch version of `tools/mpc-one/collect-over-ssh.sh`. Do not bypass MAC and SSH host-key checks or manually edit the hardcoded expected address merely to make the collector pass.
- **Ping succeeds, port 2222 refuses connections:** check whether USB autorun ran and whether `sshd` started. Inspect removable USB diagnostics after a normal shutdown, with no firmware modifications.
- **Host key mismatch after reboot:** expected with temporary keys but must still be **verified**; never unconditionally ignore the warning.
- **SSH authentication failure:** confirm that the dedicated public key on the USB matches the private key; use `ssh-add` for passphrase-protected keys.
- **Collector errors:** retain the local error output; do not edit firmware or run arbitrary fix commands.

## One-device experiment vs eventual installer

This development procedure is **not** a production installer. General users must eventually have a preflight-checked USB bootstrap with a version allowlist, individualized credentials, signed payloads, backup/restore gates, explicit physical confirmation, a reversible release selection mechanism, safe failures, and no need to run SSH. See [USB_INSTALLER_ARCHITECTURE.md](../USB_INSTALLER_ARCHITECTURE.md).
