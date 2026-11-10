# MPFree one-USB installer — staged architecture and threat model

**Status:** design, not a released installer. A working root/SSH bootstrap was demonstrated on **one** MPC One running reported firmware 3.6.0. A replacement OS or MPFree application has **not** been installed. This document specifies an eventual owner-consented, recoverable install experience, not a claim of universal root support.

## Product goal

For supported Akai standalone hardware, an end user should be able to prepare a USB stick on a computer, insert it into their MPC, approve the prompt on the hardware, and run/install MPFree without ever typing SSH commands.

**Owner direction (2026-11-09):** one click. The user inserts the USB stick in the back of the MPC, and it installs the MPFree OS/userspace and sets up an SSH server they can log into over Ethernet. Long term this includes MPFree's own Linux distribution, which may only ship once booting it is proven unable to brick the device. See [recovery and OS strategy](../research/recovery-and-os-strategy.md).

Separate these deliverables clearly:
1. **USB builder:** desktop tool that identifies the intended removable device, prepares a supported FAT filesystem and copies validated installation files. Never blindly target `/dev/sdX`; check USB transport, model, filesystem, partitions, mounted paths and device serial if available. Show destructive actions and require exact typed confirmation. Prefer *no formatting* when possible.
2. **Bootstrap:** smallest possible autorun entry point, specific to a **known supported** firmware/hardware combination. Keep the upstream validation-sensitive files immutable and separate all editable logic; do not assume it works on newer Akai builds.
3. **Preflight/installer:** separately signed, auditable application that identifies platform, verifies supported kernel/firmware ABI, checks storage, free space, filesystem constraints, original system recoverability and source content rights, then presents an on-device consent screen.
4. **MPFree userspace:** independent open-source application with its own hardware abstraction, audio graph, sampler, sequencer, interface and project storage, initially running with the existing kernel and drivers.
5. **Recovery/uninstall:** verified return to stock operation and a tested rollback path if startup, audio or storage initialization fails.

## Reversibility-first deployment modes

| Mode | Target | Persistent factory changes | Gate |
| --- | --- | --- | --- |
| A: Developer survey | temporary SSH under `/run`; no installed MPFree | None deliberately; USB diagnostic files only | **DONE** on reference MPC One |
| A+: Persistent developer SSH | early-boot SSH unit in rootfs + keys in `/data/mpfree/ssh` | **Yes: rootfs and `/data`** ([MOD-001/002](../hardware/device-modifications.md)) | **DONE** on reference unit; owner decision to keep. Not an end-user pattern: a rootfs write is Tier 3 |
| B: Portable proof of concept | RAM-backed MPFree binary/config; data from removable storage | None to internal partitions | Need actual sysroot, device map, runtime loader and executable-mount tests |
| C: Reversible MPFree userspace | versioned files on known safe storage with opt-in launch, stock OS fallback | May write a dedicated MPFree directory if explicitly authorized | Verified backups + recovery + transactional install |
| D: Independent Linux image | alternate kernel/rootfs or boot arrangement | Potentially boot-critical internal changes | **Out of scope until the Tier 3 recovery gate passes** ([recovery strategy](../research/recovery-and-os-strategy.md)) |

A portable application running on the vendor Linux base is **not** an independent firmware distribution. Use the correct terminology in every release and guide.

## Why a FAT32 USB cannot be treated as a bootable Linux root

The reference system mounted the FAT32 autorun USB with `noexec` and `nodev` semantics. Its verified autorun mechanism can source a shell script because the factory launcher interprets it, but a compiled ARM ELF program **cannot be assumed to execute directly** from this mount. Investigate RAM-backed execution (within memory limits), a separate independently writable/compatible filesystem, or a validated copy-to-runtime strategy without modifying internal data. Do not imply the stock bootloader boots USB images; we have not proved that.

## Installer state machine

~~~text
USB detected
  -> VALIDATE_UPSTREAM_AUTORUN
  -> IDENTIFY_DEVICE (exact SKU, revision, firmware, kernel, architecture)
  -> COMPATIBILITY_GATE (deny by default)
  -> VALIDATE_SIGNED_RELEASE (manifest, hashes, signatures, trusted key)
  -> STORAGE_AND_POWER_PREFLIGHT
  -> BACKUP_AND_RECOVERY_GATE
  -> SHOW_DEVICE_CONSENT
  -> STAGE_RELEASE
  -> VERIFY_STAGE
  -> LAUNCH_TEST_OR_ATOMICALLY_SELECT_RELEASE
  -> HEALTHCHECK
  -> COMMIT / AUTO-ROLLBACK
~~~

**Abort semantics:** at every failure the stock MPC remains the default boot path and the installer reports what was checked and why it stopped. “Unknown” hardware is not “probably compatible.”

### Explicitly forbidden assumptions

- That successful SSH implies privileges survive reboot or user firmware updates.
- That all MPC One / One+ / Live / Force devices share a bootloader, SoC or controller protocol.
- That read-only rootfs implies `/etc` or `/var` are nonpersistent — **on this device, both have persistent overlay upperdirs under `/data/system`**.
- That vendor audio/MIDI drivers can be opened concurrently with the factory MPC process.
- That a copied plugin library or factory sample may legally be redistributed or hosted.
- That an untested backup image constitutes a usable recovery path.

## Trust, signatures and supply chain

- Publish auditable builder source and pinned build dependencies. No network downloads from the MPC during install; prepare all artifacts on the trusted desktop.
- Generate a machine-readable manifest containing target model IDs, supported firmware hashes/versions, min storage, runtime ABI, file sizes, SHA-256 hashes and signer key ID.
- Use a deliberate, separate signing identity for MPFree releases. Pin the public verification key inside the bootstrap/build utility. Verify content before any persistent write.
- **No shared SSH private key in any image.** When remote support is explicitly requested, provision a per-device/per-user SSH public key and use non-default, key-only access. Disable it by default in end-user releases.
- Fail closed if signature verification, manifest parsing, device identity or available-space validation fails. Keep diagnostic output but exclude secrets.
- Treat the existing community autorun mechanism as a version-specific bootstrap/research method with operational and trust risks, not as a permanent security guarantee. Upstream files and licenses must be independently reviewed before packaging.

## Installer storage and recovery

The reference firmware's observed mounts:
- `/dev/mmcblk0p6` on `/` (ext4, ro);
- `/dev/mmcblk0p8` on `/data` (ext4, rw), **also** visible at `/media/az01-internal`;
- `/dev/mmcblk0p7` on `/media/acvs-content` (ext4, rw);
- persistent overlays for `/etc` and `/var` under `/data/system`.

These are **observations**, not permission to write. Before selecting a persistent installation target:

1. Record full partition table, boot chain and current update/recovery mechanism with read-only commands.
2. Determine whether an independently verified off-device image can be captured for every partition at risk; hash and preserve it.
3. Determine how recovery can actually be performed if the device no longer reaches Linux or USB autorun; verify the procedure on compatible test hardware before relying on it.
4. Prefer an isolated versioned release directory (e.g. `mpfree/releases/<version>`, **path to be chosen after investigation**), with a manifest and atomic selection pointer.
5. Never modify vendor update partitions, factory samples, license data, bootloader variables, overlay upperdirs or service definitions until a proven recovery path exists.
6. On user cancellation, report failure before changing stock startup state.

## Resource ownership and lifecycle

During development, a separate non-real-time hardware survey may run while the stock MPC GUI is active. A playback/rendering replacement will need exclusive ownership of some devices. Before a takeover experiment:

- identify exact factory application unit names, start/stop semantics and dependent services;
- save the user's project through the supported UI;
- snapshot any persistent state that might change;
- request explicit confirmation for a **reversible one-session test**;
- gracefully stop only identified owning processes, start MPFree, and restore the exact original state on timeout, crash or user exit;
- validate this lifecycle with a harmless stub before running the audio engine;
- avoid touching boot-time integration during the initial takeovers.

## Installer test plan

| Test | Expected behavior |
| --- | --- |
| Wrong USB disk or unexpected storage | Refuse to write; never auto-format user disks |
| Corrupt upstream static autorun or check file | Refuse to publish the stick |
| Changed/corrupt signed release manifest | Refuse execution |
| Unsupported MPC firmware/hardware | Refuse installation before any persistent write |
| Insufficient space or power prerequisite | Refuse installation |
| Read-only or `noexec` removable media | Choose only an explicitly validated deployment path |
| Power failure during staging | Stock OS still boots; incomplete stage ignored |
| MPFree fails health check | Restore/retain factory launch behavior |
| No reliable recovery method | Developer-mode RAM-only, never persistent install |
| No user confirmation | Never take over devices or install |
| Device with existing proprietary content | Preserve content in place; offer **local-only** optional import |

The final release must have automated tests on both host-simulated mounts and real hardware with a documented restore fixture. A working USB on one machine is evidence for one tested combination only.

## Ownership of content

MPFree can develop tools to import user-accessible projects, audio, samples and presets from a device they own, provided appropriate legal and technical constraints are respected. Ship **tools**, not extracted proprietary assets or factory binaries; do not bypass activation/licensing. Keep raw user-owned content outside the GitHub repository. See [factory-content-policy.md](factory-content-policy.md).

## Open research milestones

- Device identification (SoC + board compatible, memory, display/touch, audio/MIDI, kernel modules)
- Full eMMC partition and boot/update/recovery map; separately validated restore
- Execute-allowed filesystem and loader strategy for a portable runtime
- Effective factory application lifecycle and audio resource ownership
- Real-time audio performance budget and latency baseline
- Per-firmware autorun compatibility registry and alternate supported entry points
- Reproducible ARMv7 builds, deterministic release artifacts and signing
- End-user USB builder and one-touch consent UI

This design should evolve in response to the evidence in [hardware/baseline-3.6.0.md](../hardware/baseline-3.6.0.md), not to guesses about proprietary system internals.
