# ARMv7 hard-float userspace compatibility: cross-build versus actual az0x runtime

**Status:** The MPC One factory executable is observed ARM **ELF32 EABI5 hard-float**, with loader path `/lib/ld-linux-armhf.so.3`, on Rockchip RK3288 Linux 6.6.95 PREEMPT_RT. CI has already built independent MPFree Rust executables for `armv7-unknown-linux-gnueabihf`, but the CI sysroot is Ubuntu-based and **not qualified against the MPC's az0x 5.0.8 userspace**.

**Goal:** stop treating `cargo build --target armv7-unknown-linux-gnueabihf` as sufficient evidence of deployability. Find the exact target glibc version, dynamic loader, ABI and library availability before any nonpersistent executable is considered for on-device testing.

## Actual device results from the owner (2026-11-09 UTC)

The MPC reported ARMv7 CPU part `0xc0d` with NEON/VFPv4, working `/lib/ld-linux-armhf.so.3`, `/lib/libc.so.6`, `/lib/libm.so.6`, `libasound.so.2`, `libdrm.so.2`, EGL, GLESv2, GBM and libstdc++. Its Yocto-derived distribution is **az0x 5.0.8 (scarthgap)**.

**Crucial qualification:** `getconf` is **not installed**; the first ABI report displayed `GLIBC_2.35` through `GLIBC_2.39` from `strings /lib/libc.so.6`. Those tags **strongly suggest** a 2.39-era runtime but **do not, alone, certify the release or every exported symbol**. An earlier version of `check-arm-abi.py` exited with no target version; it must never silently promote a maximum symbol string to a confirmed version.

The revised script now prints the embedded **GNU libc release banner** from `strings` without executing libc. **Observed on this MPC:** `GNU C Library (GNU libc) stable release version 2.39.` The checker:
- uses **only an explicit getconf result or recognized embedded release banner** as a target version;
- otherwise emits **NOT QUALIFIED**, with candidate-required GLIBC version and the observed symbol-string ceiling, exit **4**;
- rejects incompatible ELF/loader or a candidate requiring newer GLIBC than an explicitly reported target, exit **2**;
- when the explicit version and needed library names are observed, reports necessary ELF/loader/version checks passed, exit **0**, **not** approval to deploy;
- exits **3** for missing/malformed reports, tools or other unhandled readelf conditions.

The sanitized measured facts and raw-report SHA-256 provenance are in [MPC One ABI/handoff survey](../hardware/abi-handoff-survey.md). **The owner ran the updated `--abi` on the actual MPC, and its embedded GNU libc release banner explicitly says 2.39.** The `--rtthreads` survey also ran and established CPU affinities across all four cores. The newest task-policy parser and passive capacity probe still require owner execution.

## New read-only userspace survey (implemented)

On ML-tower with previously verified direct MPC Ethernet and root SSH identity:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
sh -n tools/mpc-one/read-only-abi.sh
bash tools/mpc-one/collect-over-ssh.sh --abi
~~~

The script reads `uname`, CPU feature flags (without serial), `/etc/os-release`, `getconf GNU_LIBC_VERSION` if installed, **an embedded GNU libc release banner from read-only `strings`**, installed dynamic linker/library paths and symlinks for libc/ALSA/DRM/EGL/GLES/GBM/libstdc++, names of available inspection tools, optional libc GLIBC symbol tags and **only the first 64-byte header** of the factory executable (never copies the binary). It performs **no writes, no target-side compiler installation, no package changes, no ELF program execution, and no hardware device opens**.

Result: local, gitignored `research/mpc-one-abi-*.txt`. Review and redact before public sharing.

## Host-only candidate ELF comparison tool

Once the report is available, build a candidate Rust ARMv7 binary on the workstation (example below). This is **not deployment**:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
rustup target add armv7-unknown-linux-gnueabihf
sudo apt-get install --no-install-recommends \
  gcc-arm-linux-gnueabihf binutils-arm-linux-gnueabihf libc6-dev-armhf-cross
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf --workspace
~~~

Then compare a report and one local ELF with the CLI (replace the placeholder report path with the actual generated name):

~~~bash
python3 tools/mpc-one/check-arm-abi.py \
  --report research/mpc-one-abi-YOUR_REPORT.txt \
  --elf rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-render
~~~

The **host-only** checker runs cross-binutils `arm-linux-gnueabihf-readelf` to inspect the candidate ELF32/ARM/EABI5 hard-float flag, `PT_INTERP`, GNU libc versioned-symbol requirements, and `DT_NEEDED` library names. It compares the highest required GLIBC symbol version to the **actual target version** from `getconf GNU_LIBC_VERSION`. A binary requiring newer GLIBC than the device is explicitly **rejected**. Missing version or incompatible loader yields **NOT QUALIFIED**, never a guessed pass.

### What a "PASS" means and does not mean

PASS is a **necessary-version-and-header compatibility check only**, not permission to execute on the device. It does **not** prove:
- that *all* linked `DT_NEEDED` shared library names and versions are installed and behave compatibly;
- a complete target sysroot match or ABI-specific extension availability;
- CPU instruction suitability beyond advertised ARM/EABI flag (ARMv7/NEON feature flags need separate checks);
- correct audio/MIDI/DRM or USB kernel APIs;
- that a build will fit in the device's ~2 GiB memory budget or meet 128-frame realtime deadlines;
- permission or safety to copy any binary into persistent eMMC, stop `acvs.service`, take a DRM master handle, or write bootloader metadata.

A target-compatible **reproducible** sysroot should eventually be derived from *freely distributable upstream Yocto/OpenEmbedded build artifacts*, or a builder with a precisely specified libc/compiler/library ABI. If original factory libraries are privately copied for local compatibility tests, never commit or redistribute proprietary files without rights.

## Hard deployment gates

1. Owner-signed/verified source and dependency provenance; no proprietary Akai content embedded in an MPFree build.
2. Check exact DT/model, firmware version and loader/runtime, in addition to symbol comparison.
3. **Independent verified recovery** and the separate [reversible handoff plan](reversible-handoff.md).
4. Harmless **nonpersistent** executable from RAM (`/run` or `/tmp`) only after a separately reviewed and explicitly authorized staged test. No unplanned service stops or PCM opens.
5. Report true on-device results distinctly from an x86-64 GitHub runner cross-build.

## CI behavior

GitHub Actions builds the Rust workspace using the `armv7-unknown-linux-gnueabihf` target with `gcc-arm-linux-gnueabihf` plus ARM `libc6-dev` startup objects. It tests the ELF header flag, not target library compatibility. The new `check-arm-abi.py` is unit-tested with synthetic report/ELF strings, but can only run a real compatibility check after the owner supplies the **actual** read-only ABI report.

### Report-based host check, using an actual filename

When the cross-compiler/binutils are installed, cross-build on ML-tower, then:

~~~bash
python3 tools/mpc-one/check-arm-abi.py \
  --report "$(ls -t research/mpc-one-abi-*.txt | head -n 1)" \
  --elf rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-render
~~~

**This owner's updated survey confirmed the explicit GNU libc 2.39 release banner; exit 4 remains appropriate if a different device does not provide that evidence.** This is valuable evidence, not a failed MPC operation. The checker never opens SSH or executes the binary. Run this only on the workstation.

**No part of this work enables flashing, stopping the stock application, or overriding factory library search paths.**
