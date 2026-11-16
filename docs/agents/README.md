# Working on MPFree as an AI agent

**Status:** 2026-11-11. Read this before changing anything in the repository. `CLAUDE.md` at the root points here.

MPFree is built by one owner with many AI agent sessions working in parallel, each on one feature. These are the rules every session follows. Humans can read them too; nothing here is secret. Private working notes (thread handoffs, project memory, device logs) live in the private `jferments/mpfree-claude` repository, not here.

## Pages

- [How the docs stay current](docs-system.md): where a page goes, page headers, the same-PR rule, screenshots from code, the checks.
- [Writing the user manual](writing-the-manual.md): voice, structure and conventions for `docs/user/`.

## 1. Safety comes first

A mistake can brick an MPC One. For anything that touches a device:

- Every command or change given to the owner to run on the MPC is checked to be correct and reversible first, rehearsed word for word on the paths and shell it will run in (the MPC has BusyBox), with no placeholders.
- Label each step **read-only** or **state-changing**. For a state-changing step, give the exact change, how to undo it, and what happens if it fails halfway, and wait for the owner's go.
- No writes to the eMMC's boot partitions (p1 to p6, boot0/1), the U-Boot environment or the root filesystem until MaskROM recovery is proven on the unit ([recovery tiers](../dev/research/recovery-and-os-strategy.md)). Don't stop factory services without an approved gate.
- Never guess at Akai or Rockchip internals; mark assumptions as assumptions.
- **The power-on boot** (stick in at power-on starts MPFree) is explained in [How MPFree boots from the stick](../dev/hardware/soft-reboot-boot.md). Read it before changing the boot hook or the stick builder. Its invariants: the stick switches the eMMC and SD card off before udev or any mount, and powers off if it can't; every Akai unit that can touch a disk stays masked on the stick (check the build report's udev list); the hook falls back to Akai's system on any doubt and never restarts during a firmware update. With `--app-user` the pad player runs as `mpfree` (uid 960, fixed because partition 4's files carry it) with no capabilities: anything it needs root for goes in `mpfree-test-storage.service`, never in the pad player's unit. The opt-in kexec_file route and the other ways to boot (our own kernel, U-Boot) are compared in [Boot routes](../dev/hardware/boot-routes.md); a stick without `--kexec-file` must keep taking the soft-reboot unchanged. With `--chain`, keep the one-try guard (`chain-pending` on the log partition, cleared by `mpfree-test-chain-done.service` after 30 s) and keep the startup screen ordered after the chain. A `Type=oneshot` unit pulled in by `sysinit.target` needs `RemainAfterExit=yes`, or a later job runs it again in the same boot (the startup screen played twice that way). MPFree's own kernel is planned in [own-kernel.md](../dev/hardware/own-kernel.md): RAM only, built without MMC drivers.
- Never commit an eMMC backup, raw device reports, serial numbers or MAC addresses, Akai firmware, plugins, presets or sample content ([factory content policy](../dev/project/factory-content-policy.md)).

## 2. How work is split

- **One thread per feature.** Each owns its crates and the inside of its own tab or panel.
- **The app's shared layout has one owner** (the Hello MPC thread): the top bar and tab set, the Play page, the right-hand column, Prefs, and what every hardware button does. The [layout spec](../dev/design/layout-spec.md) is approved by the owner; changing it needs the owner's approval again. Other threads propose layout changes to the layout owner.
- **Performance is budgeted for the MPC One, not the workstation.** New audio code is profiled with `mpfree-prof` and must fit its budget ([profiling](../dev/design/profiling.md), [concurrency](../dev/design/concurrency.md)). Audio-thread code never allocates, locks or blocks.
- **Code pull requests merge only on the owner's explicit go.** Docs-only pull requests follow the same review unless the owner says otherwise.

## 3. Before you open a pull request

From the repository root:

```sh
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- \
  -D warnings -A clippy::needless_range_loop -A clippy::should_implement_trait
python3 docs/tools/check_docs.py
python3 docs/tools/doc_drift.py --base origin/main
```

[Testing and CI](../dev/guides/testing-and-ci.md) explains each job and the rules for tests: never skip or loosen a test to get green, new behaviour comes with tests, no test touches a device.

## 4. Docs are part of the change

A pull request that changes what a musician sees or does updates the user manual in `docs/user/`; one that changes how an engine works updates its page in `docs/dev/design/`. If no page needs to change, say why in the pull request. The details are in [how the docs stay current](docs-system.md).
