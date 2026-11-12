# MPFree third integrated owner run — QEMU disk ordering and new Ghidra evidence

Date: 2026-11-11 Pacific / 2026-11-12 UTC.

The proprietary owner ZIP and its Ghidra output are not committed to GitHub.
Private archive: `bootchain-wide-integrated-20261112T052330Z-3784948.zip`.

## Archive integrity and stage results

- ZIP CRC passed and **71 of 71 SHA-256 indexed report files verified**.
- Broad P1/P3 offline audit: passed in 54.6 s.
- V11 Ghidra updater/backend audit: passed in 47.1 s. `libaz01-prog`
  exported 137 decompiled functions; other roles succeeded.
- Independent C USB interface model: ARM static binary compiled successfully.
- A temporary read-only ext4 disk for that model was created successfully.
- QEMU guest: **failed to reach init**, not a firmware/SPL or test-binary
  execution failure; the model was never executed in the guest.
- No host USB or physical MPC devices were attached; no vendor updater
  executables were invoked.

## Root cause and correction

The new second virtio-blk-device altered guest /dev enumeration, as
directly observed in the captured QEMU serial log:

- `virtio_blk virtio0: [vda] 49152 512-byte sectors` = **24 MiB**
  temporary synthetic model ext4 fixture.
- `virtio_blk virtio1: [vdb] 881774 512-byte sectors` = **431 MiB**
  verified MPC 3.10 rootfs clone.
- The kernel still had `root=/dev/vda`, therefore selected the fixture and
  failed to execute `/bin/sh` (error `-2`); a kernel panic was expected.
- The QEMU command now selects `root=/dev/vdb` when the mock disk is
  attached and mounts the synthetic fixture read-only from `/dev/vda`.
  When the fixture is not attached, it retains the original root=/dev/vda.
- New host regression tests verify both conditions and reject unexpected
  root/device layouts. The runner now fails fast if the guest kernel
  reports `Kernel panic - not syncing`.
- **This correction is committed but still needs owner workstation execution.**
  Enumeration ordering is established for this QEMU ARM virt configuration,
  not guaranteed across unrelated QEMU machine configurations.

## New Ghidra evidence

V11 decompiled `FUN_0001d924`, called by interface scanner `0x1d9fc`.
It parses USB endpoint descriptors (type 5), uses endpoint direction
bit 0x80, stores first IN and OUT endpoint addresses in context bytes
`+0x0e` and `+0x0f`, and returns success only after both types are seen.
This helps distinguish interface selection, backend context allocation,
and endpoint discovery. It is not evidence of a physical Rockchip ROM state.

The three constructor-to-callback table relationships remain:

- class FF, subclass 42, protocol 03 -> constructor `0x16ff8`
  -> table `0x12a74`.
- class FF, subclass 61, protocol 7A -> constructor `0x1b49c`
  -> table `0x12b48`.
- class FF, subclass 06, protocol 05 -> constructor `0x1ca80`
  -> table `0x12bd8`.

The constructor at `0x1ca80` can return error -11, which the selection
logic in `0x1d9fc` treats as nonfatal and continues scanning. Further
research should examine the return-value conventions and candidate
protocol framing independently.

**Important model limitation:** The synthetic C test currently models the
interface class/subclass/protocol predicates; it does not yet model full
endpoint discovery from the updater's `0x1d924`. The next *successful* QEMU
run will validate that synthetic classifier and the guest test transport,
**not** prove the model is behaviorally equivalent to proprietary code.

## Next integrated experiment

Pull `research/wide-bootchain-audit-20261111` and run the existing
single-command static + QEMU orchestrator. After the guest successfully
executes the synthetic model, extend the model and tests to include proper
IN/OUT endpoint recognition and malformed-descriptor cases. Keep all
physical bootloader flashing, OTP, USB recovery claims, and eMMC restore
outside this experiment until independent hardware proof exists.
