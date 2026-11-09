# mpfree

**A proposed free, open-source music-production environment for the Akai MPC One.**

**Status: design / hardware discovery.** This repository does **not** yet contain a working replacement application or firmware image.

## Vision

An offline-first, hardware-focused sampler, sequencer, arranger, synth/effects host, and production workstation that runs on the MPC One's Linux platform. Keep the tactile MPC workflow while introducing flexible clip and timeline arrangements, polymeter, parameter locks, conditional triggers, better automation, and a less menu-heavy UI.

## Starting point

We are beginning with a **userspace application**, not a replacement kernel, bootloader, or destructive firmware installer. We will first validate the hardware and build a reversible demonstration that can read a pad and output audio.

Read:
- [Architecture proposal](docs/ARCHITECTURE.md)
- [Roadmap and milestone gates](docs/ROADMAP.md)
- [Factory content and plugin compatibility policy](docs/FACTORY_CONTENT.md)

A read-only [hardware survey script](tools/collect-hardware-info.sh) is available for manual execution on a device you own or are authorized to inspect. Review and redact its output before sharing.

## Legal and technical boundaries

mpfree will not distribute proprietary Akai/AIR firmware, plugin binaries, presets, or factory sample packs, or bypass activation. Optional compatibility with legitimately user-owned content can be researched independently. The original factory environment must remain recoverable.

## License

**Not selected yet.** We intend to use an OSI-approved free/open-source license, but do not assume that this repository is licensed for reuse until a license file is added.

## Research starting points

- [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore)
- [TheKikGen/MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs)

These are independent projects; review their licenses and hardware applicability before incorporating code.
