# Integrated offline boot-chain research: first bundled run

Date: 2026-11-11 Pacific (archive timestamp 2026-11-12 UTC).

This public-source-only note summarizes **owner-private** evidence received in
`bootchain-wide-integrated-20261112T040216Z-3753157.zip`. The proprietary
Ghidra output, firmware ELF/rootfs bytes, console transcript and private ZIP
are NOT stored in this repository.

## Integrity and status

- Full ZIP CRC passed; all **82 SHA-256 indexed files** verified.
- Broad Ghidra audit: passed; V11 Ghidra updater + SPL/U-Boot audit: passed.
- SPL: 213/213 Ghidra-recognized functions covered; 19 computed call sites.
- U-Boot: 1050/1050 Ghidra-recognized functions covered; 167 computed call sites.
- These figures **do not** prove all code and indirect edges are recovered.
- QEMU Linux 7.0.14 booted the copied 3.10 rootfs; guest ran all scripted
  passive inspection commands. The serial transcript contains the final
  `MPFREE_VM_DONE_...` marker. The runner incorrectly reported timeout due
  to QEMU PL011/PTTY CRCRLF formatting (fixed in the research runner).
- Guest sha256 of `/usr/sbin/az01-update` and
  `/usr/lib/libaz01-prog.so.1.3` exactly matched the source hashes
  used for V11 static Ghidra imports.
- QEMU's dynamic linker resolved updater dependencies; this does **not**
  constitute execution of updater application logic.

## Evidence relevant to independent boot

1. The 3.6/3.10 normal U-Boot scripts are equivalent after normalization.
   In the compared board DTBs, 3.10 adds `mode-update=0x5242c301` to
   `syscon-reboot-mode`, aliasing the existing `mode-loader` value. No
   evidence of a distinct silicon MaskROM entry follows from this alias.
2. V11 finds **seven overlapping candidate callback table bases** in
   `libaz01-prog`. The most interesting candidates are ELF vaddrs
   `0x12a74` (marker 2, six matching slots), `0x12b48` (marker 4,
   five slots), and `0x12bd8` (marker 1, four slots).
3. Ghidra observes references to candidate table regions from code
   addresses including `0x17044`, `0x1b4e8`, `0x1cad0`, but
   **has not proved context assignment or the selected runtime backend**.
   The updated V11 pass now decompiles enclosing source functions and
   records referencing instruction neighborhoods for these table xrefs.
4. `az01_verify_image` at ELF vaddr `0x17554` caches result state:
   state 1 succeeds, `-13` assigns state 2, and a later verification
   with flag 2 accepts a nonzero cached state. The write-image function
   uses verification flag 2. Reachability to any real write path in the
   state-2 case remains **unproven**; this is NOT a verified signature bypass.
5. Linux userspace QEMU tests **do not execute** the physical RK3288
   BootROM, SRAM SPL or stock U-Boot, and cannot prove physical ROM
   download, unsigned owner SPL execution, secure-boot enforcement,
   hardware restore or eMMC write safety.

## Next experiment

Run the single integrated Ghidra/QEMU orchestrator again after fetching
`research/wide-bootchain-audit-20261111`.

- Run the included synthetic serial completion regression tests.
- Examine newly decompiled callback-table xref sources for direct writes
  to backend pointers, selection based on USB state and format-table
  false positives.
- Use the guest inventory for controlled future mock-device and
  synthetic-image tests. Do not launch vendor update/flash/OTP utilities
  until an isolated outer x86 KVM VM plus rejecting fake-device interfaces
  are implemented and tested.
- Separate hardware gate: genuine independent MaskROM entry, SRAM
  owner-loader execution, and offline-verified full eMMC readback/restore
  remain unproven. No boot-critical storage writes are authorized.

The integrated orchestrator packages the source audit ZIPs, full stage
console output, guest serial output, cross-evidence summary and SHA index
into one owner-private ZIP. It deliberately does not upload private
artifacts to GitHub.
