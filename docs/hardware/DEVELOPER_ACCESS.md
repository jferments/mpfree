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

### Verified first inventory and the focused follow-up

The reference device's first baseline inventory succeeded on **2026-11-09 05:00:46 UTC** after loading the dedicated private key into `ssh-agent`. Local report SHA-256: `4172d29e33359eee106cfcf03665d3a94cd4e4deed75d009a1fe1065ea53850b`. It revealed the Rockchip RK3288 SoC, real eMMC partition labels, ACVA ALSA, three USB MIDI endpoints, DRM/Panfrost and ILI2117 input, and the real factory application unit **`acvs.service`**. See [the redacted field report](MPC_ONE_3_6_0_SURVEY_20261109.md).

**First collector limitation:** BusyBox rejected some GNU-style utilities; `ps -eo ... | head` produced empty output without activating its fallback, the expected `inmusic-mpc.service` did not exist, and `aplay`/`arecord`/`aconnect` were not installed. New builds use `/proc` for process names, query `acvs.service`, distinguish missing userspace tools, and **redact the unique `Serial:` field from `/proc/cpuinfo`**.

Run on the **workstation** after fetching the latest existing PR #1 branch; no need to recreate the USB or network:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash -n tools/mpc-one/collect-over-ssh.sh
sh -n tools/mpc-one/read-only-detail.sh
bash tools/mpc-one/collect-over-ssh.sh --detail
~~~

The `--detail` mode uses the *same strict SSH host-key, wired interface, route and MPC MAC checks* as the baseline. It sends only the repository-shipped `read-only-detail.sh` program. The narrowed survey reads running process and service metadata, hardware fd **symlinks** (does not open devices), ALSA card metadata, display modes, input capabilities, USB vendor/product descriptors excluding serials, CPU frequency and a short executable ELF header. It does **not** control audio, transmit MIDI, stop services or write internal storage.

The report is written locally under `research/mpc-one-detail-<timestamp>-<random>.txt`, with a companion SHA-256 file. Always review/redact report contents before posting or uploading anywhere. The original baseline report **contained a private hardware serial** and is **not** stored in Git.

### Verified second survey and next audio/thread probe

The focused collector **succeeded** at approximately `2026-11-09 05:21:06 UTC`. Its owner-local report SHA-256 is `d25be98aea6e90cc0328c0af4d29c505106dcd26704da7dc1f6f5bd59cec317e`. See [redacted service and hardware resource map](MPC_ONE_3_6_0_RESOURCES_20261109.md): `acvs.service` owns ALSA PCM playback and capture, raw MIDI and sequencer, DRM card0, touchscreen event0 and gpio-keys event1; GPU identifies as Panfrost/Mali-T760 compatible; native DSI mode is 800x1280. The uploaded raw report remains **off-GitHub**.

**Known probe bug fixed:** BusyBox rejected `head -c` and `od -An` while reading the 64-byte ELF header. The corrected probe uses a read-only `dd if=/usr/bin/MPC bs=64 count=1 | od -b` command to inspect only a header, not copy a vendor executable. This fix is not evidence that the ELF header was successfully captured yet.

The follow-up `--runtime` script reads ALSA's **current** PCM `hw_params` and `sw_params` from procfs, the actual factory process's thread scheduling fields, the main process's memory `VmLck`, USB MIDI interface class metadata, and the first 64 bytes of its ELF header. It does **not** open device handles, alter sample rates, stop applications, control buttons or MIDI, change mixer settings, or write to the MPC.

Run these **exact commands on ML-tower** with the established link:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash -n tools/mpc-one/collect-over-ssh.sh
sh -n tools/mpc-one/read-only-runtime.sh
bash tools/mpc-one/collect-over-ssh.sh --runtime
~~~

Local output: `research/mpc-one-runtime-<UTC-timestamp>-<suffix>.txt` and `.txt.sha256`.

**Do not confuse these data sources:** procfs `hw_params` describes the **currently configured** PCM stream while it is open, not the driver's complete supported rate/format matrix. The thread policy snapshot likewise describes only the present firmware runtime, not a promised realtime configuration on every device.

### Third survey: confirmed live audio engine baseline (successful)

The third read-only runtime survey succeeded at approximately **2026-11-09 05:30:31 UTC**. Report SHA-256: `1ea9eb27f837e5602cd5548db71622adf2f8c6aef52553f2aafd22e035a34b76`, verified against the uploaded owner-local report bytes. The raw report is **not** committed to GitHub.

The stock `/usr/bin/MPC` process exposed:
- Active playback and capture both **44,100 Hz, stereo, interleaved S32_LE**, **128 frames/period**.
- Playback ALSA buffer **384 frames**; capture buffer **256 frames**. These durations do **not** establish real-world latency.
- ~1 GiB resident and locked virtual memory, 38 threads, main thread affinity CPU 0.
- An ALSA kernel RawMIDI counter table and explicit JUCE/MPC sequencer connections.
- ARM ELF32 little-endian `EM_ARM`, EABI5 **hard-float** flag `0x05000400`.

The thread-policy query returned names but **not** the policy/priority fields; do not claim specific realtime priorities or per-thread CPU affinities. The firmware's `acvs.service` reports `Restart=always`, so a simple `kill` is not a safe takeover test.

Full reproducible evidence, interpretation, tables and missing capabilities: [third survey](MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md). This provided the parameters for [MPFree's first separate host-only metronome core](../engineering/FIRST_AUDIO_CORE.md).

### Completed Ethernet/USB research run (2026-11-09 UTC)

The owner's read-only `--io` and explicitly opt-in active `--netbench` experiments have both **succeeded**. See [the redacted measured reference](MPC_ONE_3_6_0_ETHERNET_USB_20261109.md).

Source-report SHA-256:
- Passive `--io`: `92687a46ab7d04d81ba6bf05486e29e6f67f73e66c0c9ea0ad7e136e20eee45d`.
- Active `--netbench`: `6d206eba95640a385b58ea76f36ea91723591b44fada7155a5fff9de14679d01`.

Directly observed: `end0` negotiated **1 Gbit/s full-duplex** via `rk_gmac-dwmac` + **Realtek RTL8211E**; zero error/drop counters at that sampling time; 20/20 ICMP responses, average 0.272 ms; short 4 MiB SSH goodput **126.66 Mbit/s MPC→workstation**, **47.05 Mbit/s workstation→MPC**. The PNY flash drive uses **USB 2.0 high-speed (480 Mbit/s)**, and USB gadget UDC `ff580000.usb` is registered high-speed but not attached. This is *not* a validated USB networking or PCM audio stream.

The general scripts below remain useful for other authorized developers using their own independently identified network settings. **Do not repeat the active benchmark during live music work**.

### New interface survey: Ethernet and USB, plus optional bounded data-transfer benchmark

The owner requested research into using a connected Linux laptop or desktop as an **auxiliary DSP computer**, sending generated audio back to an MPC. The architecture, raw PCM bandwidth/latency model and hardware limitations are documented in [Auxiliary Compute and I/O](../engineering/AUXILIARY_COMPUTE_AND_IO.md).

**First run the passive probe** from ML-tower on the same established Ethernet/SSH configuration:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash -n tools/mpc-one/collect-over-ssh.sh
sh -n tools/mpc-one/read-only-io.sh
bash tools/mpc-one/collect-over-ssh.sh --io
~~~

The passive `--io` mode checks the wired `end0` link rate/duplex, PHY and driver metadata, interface error counters, enumerated USB-device speeds, host-controller bindings and whether a Linux USB device/gadget controller is registered. It **does not** change link speed, mount/remount storage, open audio devices or start listeners. All output is under locally ignored `research/mpc-one-io-*.txt`.

**Then, only if the MPC is idle and you're comfortable with a small CPU/network load, optionally run:**

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
bash tools/mpc-one/collect-over-ssh.sh --netbench
~~~

This is an **active** test; it performs 20 ICMP pings and transfers **4 MiB in each direction over key-authenticated, non-compressed SSH**. The remote only reads `/dev/zero` or writes to `/dev/null` (a sink, not a storage filesystem). It does not install software, alter the MPC OS or open ALSA, MIDI or DRM. It can temporarily burden CPU/network, so **do not run while playing, recording or performing**. The benchmark explicitly reports encrypted SSH throughput with handshake/crypto cost, **not** raw wire speed. Reports are saved in `research/mpc-one-netbench-*.txt`.

Both commands retain the same verified `eno1np0`/MPC MAC identity gates and strict SSH host-key checking as previous successful probes. **The new probes still require first real-device verification**; CI shell syntax and safety heuristics are not proof that firmware exposes every optional file.

### M3: read-only ABI and reversible-handoff preflights (first run still pending)

The current research is moving toward a **Rust-first userspace** without replacing the stock kernel. Before testing an MPFree executable on the MPC, we must establish the **actual GNU libc version and dynamic loader compatibility**, and map the **real stock application lifecycle and resource ownership**.

Both commands below use the same direct Ethernet IP/MAC/host-key identity check and SSH key as the successful previous hardware surveys. They **do not** stop `acvs.service`, open ALSA/DRM/MIDI, modify firmware, write persistent storage, or alter USB controller roles:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
sh -n tools/mpc-one/read-only-abi.sh
sh -n tools/mpc-one/read-only-handoff-preflight.sh

bash tools/mpc-one/collect-over-ssh.sh --abi
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

Local reports are saved under `research/mpc-one-abi-*.txt` and `research/mpc-one-handoff-preflight-*.txt`, plus SHA-256 sidecars. **Redact boot args, local IP/MAC, serials and unit environment information before public sharing**; raw reports must stay off GitHub.

`--abi` lists user-space loader/libc/ALSA/DRM/EGL paths and glibc version. [ARM compatibility guide](../engineering/ARMV7_USERSPACE_COMPATIBILITY.md) explains how to use the host-only ELF comparison script after receipt. `--handoff-preflight` maps systemd unit behavior, fds and rollback dependencies; [reversible handoff design](../engineering/REVERSIBLE_HANDOFF.md) explicitly prohibits any live takeover until independent recovery and a watchdog-backed restoration test are proven. **Neither new probe has been run on the owner's MPC yet.**

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
