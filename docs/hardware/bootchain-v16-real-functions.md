# MPFree V16: bounded execution of real vendor bootloader functions

## The user-requested one-command research run

From the separate research repository checkout on ML-tower:

```bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v16-real-functions && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v16.py" \
  "$R/tools/mpc-one/evidence_v16.py" \
  "$R/tools/ghidra/MPFreeV16LiveStatic.py" \
  "$R/tools/mpc-one/test_v16.py" && \
arm-linux-gnueabihf-gcc -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v16.c" && \
python3 "$R/tools/mpc-one/test_v16.py" && \
python3 "$R/tools/mpc-one/run_v16.py"
```

The runner auto-selects **SHA-indexed original private** V14 and V15 research
ZIPs from `~/mpfree-work/`, and refuses mismatched owner binary hashes. Override
with `--prior-v14 PATH --prior-v15 PATH`; use `--offline-only` for reproducible
85-question static corpus reassessment without host QEMU/Ghidra.

## New experiments and limits

- Concurrent Ghidra P1/P3 checks the actual owner SHA-pinned saved projects,
  and P3 exports a **temporary private verified complete image** into a
  cleanup-controlled directory. This image never enters the final ZIP.
- The guest maps actual P3 ARM instructions into its own user-mode RAM.
  A *stub* replaces the vendor USB response, and the original device and
  updater are never opened or executed. No network or physical USB passthrough.
- Instead of seven repeats of a tiny permission function, the guest executes
  **more than 100 distinct cases** across actual vendor **OEM dispatch**,
  authorization, bounded string comparison, Fastboot RX payload copying,
  image classification, callback installation and USB exit-state setting.
  It includes all 37 individual-byte mutations and all 38 truncation lengths
  of the OEM authorization string using values extracted privately from the
  verified image. The authentication token is never printed or archived.
- Every case reports its actual result, including unexpected behavior or a
  crash. The script does not mark missing results as passing.
- 85 prior questions are re-examined against the actual V14/V15 code, with
  named code paths and defensible classifications. **Functions looked at !=
  questions answered.** Hardware-specific questions remain unresolved.
- The entire suite uses the owner's preexisting ARM `virt` QEMU clone and
  saved Ghidra projects, running with `-snapshot -nic none -monitor none`.
  This is **NOT** an RK3288 SoC simulator and cannot establish actual
  MaskROM, SD priority, eFuse, eMMC recovery or independent kernel boot.

## Output

ONE private SHA-indexed research ZIP:

`~/mpfree-work/bootchain-wide-<UTC>-v16-<PID>.zip`

It includes measured stage timing, dataflow, raw guest serial, 100+ case
results, and the complete 85-question evidence ledger, but **not** binary
firmware, ext4 images, raw owner partitions, guest executables or secrets.

## Prerequisites

Previously configured saved P1/P3 Ghidra projects and disposable 3.10 ARM
QEMU clone in `~/mpfree-work` plus `arm-linux-gnueabihf-gcc`,
`qemu-system-arm`, `qemu-img`, `mkfs.ext4`, and Ghidra Snap or `GHIDRA_HOME`.
The first run does not auto-install or change any host packages.

## What is and is not proven

This V16 runner is designed to test **real** vendor instruction paths under
mock memory and software USB stubs. Successful experiments prove *only* the
emulated path under those mock conditions. There are no physical Fastboot
commands, no bootloader unlock, no flash/erase, no `saveenv`, no OTP/eFuse,
no physical MaskROM entry and no original file modifications.

If any individual experiment fails, its JSON row and serial transcript are
kept. A stage failure does not suppress the single final ZIP.