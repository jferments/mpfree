# MPFree V19 — Recovery, ADC, GETVAR and typed P-code boot-path analysis

Branch: research/bootchain-v19-recovery-getvar-20261112

## V18 results that V19 builds upon

The SHA-verified owner V18 research archive passed integrity verification.
Its 41.1-second run decompiled 213/213 P1 and 1044/1050 P3 recognized
functions. It examined 210,005 High P-code operations and generated
4,483 STORE/CALLIND/BRANCHIND records. Six remaining P3 errors concern
Ghidra address-space boundaries.

V18's basic constant resolver found only five possible storage destination
values and two candidate indirect calls. These are NOT proof that no
other memory writes or indirect callbacks exist.

The ARM VM ran 16 novel cases, with 14 passes and two segmentation
faults in GETVAR variant 4 (both locked and unlocked). The owner's real
P3 getvar handler invokes an environment lookup on this branch, but V18
did not mock it. This must be investigated before treating it as a
physical MPC crash or U-Boot memory vulnerability.

## One copy-paste command on the ML-tower

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v19-recovery-getvar-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v19.py" \
  "$R/tools/mpc-one/test_v19.py" \
  "$R/tools/ghidra/MPFreeV19PathWitness.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v19.c" && \
python3 "$R/tools/mpc-one/test_v19.py" && \
python3 "$R/tools/mpc-one/run_v19.py"
~~~

The script auto-selects the newest SHA-indexed V18 owner-private ZIP in
~/mpfree-work. Override with --prior-v18 /absolute/path/to/archive.zip.

Output: exactly ONE owner-private
~/mpfree-work/bootchain-wide-UTC-v19-PID.zip
with complete SHA-256 index and a separate checksum sidecar.

## Genuine concurrent investigations

1. Full saved-project P1/SPL recursive call graph and fine-grained
   typed source dependency traces: first-stage media preference,
   ROM handoff, SRAM and recovery. Every recognized function reachable
   by direct calls (forward and reverse, to fixed point) is indexed;
   functions in a relevant four-hop neighborhood receive new detailed
   High P-code STORE/CALLIND/BRANCHIND expression analysis.

2. Full saved-project P3/U-Boot recursive call graphs and new typed
   High P-code traces for environment GETVAR, boot-counter/reboot-mode,
   ADC recovery selection, OEM auth, command interpreter, storage
   backend callbacks, bootm, image loading and SD/MMC boot selection.
   Static pointer cells and their contents remain distinct from
   live runtime RAM values. Unknown pointer inputs remain unknown.
   Expression trees are bounded against PHI explosion and preserve
   the truncation reason.

3. NEW actual ARM vendor execution: run original P3 code at the
   recovery-selection function with guest-only fake reboot-mode
   registers, recovery-flag RAM and ADC callbacks. Exercises modes
   0/1/3/9/255, a bad reboot-mode prefix, and ADC values
   0/1/99/100/101/255. Checks which cases select recovery and whether
   the fake PMU register is cleared.

4. NEW actual ARM vendor GETVAR branch repair: V18's variant 4
   crashed because the genuine owner getvar path called a live U-Boot
   environment accessor in the synthetic guest. V19 intercepts
   only that accessor and returns null/empty/nonempty fake values
   while recording call counts. Runs authorized/unauthorized variants
   and variant 5 control cases.

5. All original 85 questions are carried forward and linked to
   specific new code witnesses and isolated VM evidence only if that
   corresponding stage actually completed. No keyword counting;
   no claim that QEMU emulates physical RK3288 BootROM.

There are 24 NEW independent guest process scenarios. They are not
24 different open research questions. Earlier 1,239 V17 and 16 V18
test cases are not repeated.

## Safety

Input SHA-pinned owner's P1/P3 firmware and saved Ghidra projects remain
unchanged. Projects are processed -readOnly -noanalysis. The P3 binary
copy exists only in a private temporary directory and is destroyed
after the QEMU guest run. The QEMU guest uses a cloned rootfs under
-snapshot, with network and host USB/MMC passthrough disabled. Only
original P3 functions, with hardware/storage callbacks replaced in
guest memory, are executed. No physical MPC access, official updater,
USB OEM command, flash, erase, saveenv, real MMC I/O, BootROM or eFuse.

The output ZIP contains detailed reports only, NOT private firmware
or updater binary code. All source reports are owner-private.

## Verification standard

The branch and regression tests are committed, but V19 has NOT yet run
in the owner's Ghidra/QEMU environment. The copy/paste command compiles
and regression-tests its Python and ARM C sources before launching work.
Stage results will be judged from actual function coverage, specific
typed witnesses, full guest console and per-case observations. Failed
VM or Ghidra stages remain failed and are never counted as answers.
