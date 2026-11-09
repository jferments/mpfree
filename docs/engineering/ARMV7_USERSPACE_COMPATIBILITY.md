# ARMv7 hard-float userspace compatibility: cross-build versus actual az0x runtime

**Status:** The MPC One factory executable is observed ARM **ELF32 EABI5 hard-float**, with loader path `/lib/ld-linux-armhf.so.3`, on Rockchip RK3288 Linux 6.6.95 PREEMPT_RT. CI has already built independent MPFree Rust executables for `armv7-unknown-linux-gnueabihf`, but the CI sysroot is Ubuntu-based and **not qualified against the MPC's az0x 5.0.8 userspace**.

**Goal:** stop treating `cargo build --target armv7-unknown-linux-gnueabihf` as sufficient evidence of deployability. Find the exact target glibc version, dynamic loader, ABI and library availability before any nonpersistent executable is considered for on-device testing.

## New read-only userspace survey (implemented)

On ML-tower with previously verified direct MPC Ethernet and root SSH identity:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
sh -n tools/mpc-one/read-only-abi.sh
bash tools/mpc-one/collect-over-ssh.sh --abi
~~~

The script reads `uname`, CPU feature flags (without serial), `/etc/os-release`, `getconf GNU_LIBC_VERSION` if installed, installed dynamic linker/library paths and symlinks for libc/ALSA/DRM/EGL/GLES/GBM/libstdc++, names of available inspection tools, optional libc GLIBC symbol tags and **only the first 64-byte header** of the factory executable (never copies the binary). It performs **no writes, no target-side compiler installation, no package changes, no ELF program execution, and no hardware device opens**.

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
3. **Independent verified recovery** and the separate [reversible handoff plan](REVERSIBLE_HANDOFF.md).
4. Harmless **nonpersistent** executable from RAM (`/run` or `/tmp`) only after a separately reviewed and explicitly authorized staged test. No unplanned service stops or PCM opens.
5. Report true on-device results distinctly from an x86-64 GitHub runner cross-build.

## CI behavior

GitHub Actions builds the Rust workspace using the `armv7-unknown-linux-gnueabihf` target with `gcc-arm-linux-gnueabihf` plus ARM `libc6-dev` startup objects. It tests the ELF header flag, not target library compatibility. The new `check-arm-abi.py` is unit-tested with synthetic report/ELF strings, but can only run a real compatibility check after the owner supplies the **actual** read-only ABI report.

**No part of this work enables flashing, stopping the stock application, or overriding factory library search paths.**
