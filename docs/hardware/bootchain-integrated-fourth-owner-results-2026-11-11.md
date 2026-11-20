# MPFree fourth integrated owner run — two-disk QEMU and synthetic ARM program proven

**Owner-private archive:** `bootchain-wide-integrated-20261112T055817Z-3799915.zip`.
The original archive, proprietary Ghidra pseudocode, ELF bytes, and extracted
firmware are not committed to this public repository.

## Integrity and run status

ZIP CRC passed; **84/84 indexed files** passed SHA-256 verification.
The integrated session finished in **105.2 seconds**, with no reported
stage failures:

| Component | Result |
| --- | --- |
| Broad offline SPL/U-Boot Ghidra | Pass, 54.8 s |
| Targeted V11 updater/backend Ghidra | Pass, 47.2 s |
| Static ARM USB interface model compilation | Pass |
| Synthetic ext4 fixture creation | Pass |
| ARM QEMU Linux guest boot, root=/dev/vdb | Pass |
| Read-only mount of model fixture /dev/vda | Pass |
| ARM model executed in QEMU | **13/13 cases pass** |
| QEMU serial command acknowledgments | **39/39** |
| Guest vs static updater and library SHA-256 | Exact matches |

The guest used an isolated, cloned MPC 3.10 root filesystem on the
generic QEMU ARM `virt` machine with Linux 7.0.14. It did not boot the
RK3288 BootROM, original SPL or stock U-Boot. The QEMU lab's root
filesystem lives on a virtual drive, and its secondary synthetic fixture
is a temporary, read-only regular-file disk. No physical device/USB
passthrough, firmware-writing operation, vendor updater process or
physical MPC contact took place.

The 13-case model tests three interface class/subclass/protocol
combinations along with ten unmatched or malformed cases. All passed,
including JSON case records and explicit `MPFREE_MODEL_EXEC_OK` and
`MPFREE_USB_DESCRIPTOR_MODEL_PASS` markers in the guest serial log.
**This verifies our independently written model and ARM guest harness,
not that the original vendor parser behaves identically.**

## Static knowledge and remaining gates

The verified V11 stage reported 12 decompiled `az01-update` functions,
137 `libaz01-prog` functions, 25 directed SPL functions and 55 directed
U-Boot functions. The broader Ghidra stages still identify 213 SPL and
1050 U-Boot functions, and their unresolved computed-call and boot-handoff
paths remain. Three USB interface conditions are correlated with callback
tables, but protocol identity and actual device runtime modes remain
unconfirmed. The Linux guest's `kexec_load_disabled=0` belongs to the
**QEMU kernel** and says nothing about physical MPC boot policy.

The critical hardware gates are still independent proof of actual
silicon MaskROM entry, unsigned owner-controlled SRAM loader acceptance,
and verified eMMC recovery/restore. Neither this QEMU result nor static
Ghidra analysis establishes them.

## New next experiment (committed, not yet owner-run)

`tools/mpc-one/usb_endpoint_mock_model.c` is a second independently
written C model based on the previously identified endpoint scanner
at `libaz01-prog` ELF address `0x1d924`. It uses defensive descriptor
length checks, recognizes the **first IN and first OUT endpoints**, and
declares success only after both are present. It tests 21 synthetic
configurations: reversed direction order, missing or duplicate endpoints,
short/truncated/malformed descriptors, interposed unrelated descriptors,
adjacent interfaces and different endpoint attributes.

The new model was compiled and **21/21 cases passed locally** with the
host C compiler. The integrated script now cross-compiles *both*
programs for ARM, packs them into the same temporary read-only ext4
fixture, executes both sequentially inside the QEMU guest, and stores
their per-case JSON results and distinct pass/fail markers in the usual
single private output ZIP. The first 13-case model is retained.

This second model is **a safe, independently implemented approximation**.
The original scanner's precise malformed-input behavior is not proven,
and the model does not simulate the actual Rockchip USB protocol, firmware
updates, or hardware recovery.

## Research direction

After proving both synthetic tests in QEMU, prioritize recovering
transport framing and return-state handling for the three specific
programming backends. Keep a strict separation between:

1. Reconstructed/proven **static firmware facts**.
2. Independently written **synthetic model tests** in generic ARM QEMU.
3. Tests of original firmware code inside an **additional isolated** VM
   with entirely fake/rejecting device interfaces.
4. Real hardware proof and recovery checks, not attempted by this runner.
